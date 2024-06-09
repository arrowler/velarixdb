use crate::bucket_coordinator::BucketMap;
use crate::consts::FLUSH_SIGNAL;
use crate::types::{self, FlushSignal};
use crate::{
    bloom_filter::BloomFilter, cfg::Config, err::StorageEngineError, key_offseter::KeyRange,
    memtable::InMemoryTable,
};
use futures::lock::Mutex;
use indexmap::IndexMap;
use std::sync::Arc;
use tokio::sync::RwLock;

type K = types::Key;

pub type InActiveMemtableID = Vec<u8>;
pub type InActiveMemtable = Arc<RwLock<InMemoryTable<K>>>;
pub type FlushDataMemTable = (InActiveMemtableID, InActiveMemtable);

use tokio::spawn;
use tokio::sync::mpsc::Receiver;

#[derive(Debug)]
pub struct FlushUpdateMsg {
    pub flushed_memtable_id: InActiveMemtableID,
    pub buckets: BucketMap,
    pub bloom_filters: Vec<BloomFilter>,
    pub key_range: KeyRange,
}

#[derive(Debug)]
pub enum FlushResponse {
    Success {
        table_id: Vec<u8>,
        updated_bucket_map: BucketMap,
        updated_bloom_filters: Vec<BloomFilter>,
        key_range: KeyRange,
    },
    Failed {
        reason: StorageEngineError,
    },
}

#[derive(Debug, Clone)]
pub struct Flusher {
    pub(crate) read_only_memtable: Arc<RwLock<IndexMap<K, Arc<RwLock<InMemoryTable<K>>>>>>,
    pub(crate) bucket_map: Arc<RwLock<BucketMap>>,
    pub(crate) bloom_filters: Arc<RwLock<Vec<BloomFilter>>>,
    pub(crate) key_range: Arc<RwLock<KeyRange>>,
    pub(crate) use_ttl: bool,
    pub(crate) entry_ttl: u64,
}

impl Flusher {
    pub fn new(
        read_only_memtable: Arc<RwLock<IndexMap<K, Arc<RwLock<InMemoryTable<K>>>>>>,
        bucket_map: Arc<RwLock<BucketMap>>,
        bloom_filters: Arc<RwLock<Vec<BloomFilter>>>,
        key_range: Arc<RwLock<KeyRange>>,
        use_ttl: bool,
        entry_ttl: u64,
    ) -> Self {
        Self {
            read_only_memtable,
            bucket_map,
            bloom_filters,
            key_range,
            use_ttl,
            entry_ttl,
        }
    }

    pub async fn flush(
        &mut self,
        table: Arc<RwLock<InMemoryTable<K>>>,
    ) -> Result<(), StorageEngineError> {
        let flush_data = self;
        if table.read().await.entries.is_empty() {
            println!("Cannot flush an empty table");
            return Err(StorageEngineError::FailedToInsertToBucket(
                "Cannot flush an empty table".to_string(),
            ));
        }

        let table_bloom_filter = &mut table.read().await.bloom_filter.to_owned();
        let table_biggest_key = table.read().await.find_biggest_key()?;
        let table_smallest_key = table.read().await.find_smallest_key()?;
        let hotness = 1;
        let sstable_path = flush_data
            .bucket_map
            .write()
            .await
            .insert_to_appropriate_bucket(&table.read().await.to_owned(), hotness)
            .await?;

        let data_file_path = sstable_path.get_data_file_path().clone();

        flush_data.key_range.write().await.set(
            data_file_path,
            table_smallest_key,
            table_biggest_key,
            sstable_path.clone(),
        );

        table_bloom_filter.set_sstable_path(sstable_path);
        flush_data
            .bloom_filters
            .write()
            .await
            .push(table_bloom_filter.to_owned());

        // sort bloom filter by hotness
        flush_data.bloom_filters.write().await.sort_by(|a, b| {
            b.get_sstable_path()
                .get_hotness()
                .cmp(&a.get_sstable_path().get_hotness())
        });

        Ok(())
    }

    pub fn flush_handler(
        &mut self,
        table_id: Vec<u8>,
        table_to_flush: Arc<RwLock<InMemoryTable<K>>>,
        flush_signal_sender: async_broadcast::Sender<FlushSignal>,
    ) {
        let flush_signal_sender_clone = flush_signal_sender.clone();
        let buckets_ref = self.bucket_map.clone();
        let bloomfilter_ref = self.bloom_filters.clone();
        let key_range_ref = self.key_range.clone();
        let read_only_memtable_ref = self.read_only_memtable.clone();
        let use_ttl = self.use_ttl;
        let entry_ttl = self.entry_ttl;
        spawn(async move {
            let mut flusher = Flusher::new(
                read_only_memtable_ref.clone(),
                buckets_ref,
                bloomfilter_ref,
                key_range_ref,
                use_ttl,
                entry_ttl,
            );
            let ttt = table_to_flush.clone();
            println!(
                "Number of entries before flush {}",
                ttt.read().await.clone().get_index().len()
            );
            match flusher.flush(table_to_flush).await {
                Ok(_) => {
                    read_only_memtable_ref.write().await.shift_remove(&table_id);
                    let flush_signal_sender_clone2 = flush_signal_sender_clone.clone();
                    println!("Notgification sent=============================================================1");
                    tokio::spawn( async move {
                        let broadcase_res = flush_signal_sender_clone2.try_broadcast(FLUSH_SIGNAL);
                        match broadcase_res {
                            Ok(_) => {println!("Notgification sent=============================================================2")}
                            Err(err) => match err {
                                async_broadcast::TrySendError::Full(_) => {
                                    log::error!("{}", StorageEngineError::FlushSignalOverflowError)
                                }
                                _ => log::error!("{}", err),
                            },
                        }
                    });
                    
                }
                // Handle failure case here
                Err(err) => {
                    println!("Flush error: {}", err);
                }
            }
        });
    }
}
