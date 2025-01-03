#[cfg(test)]
mod tests {
    use crate::consts::{SIZE_OF_U32, SIZE_OF_U64, SIZE_OF_U8};
    use crate::db::{DataStore, SizeUnit};
    use crate::err::Error;
    use crate::gc::garbage_collector::GC;
    use crate::types::Key;
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::sync::RwLock;

    async fn setup(
        store: Arc<RwLock<DataStore<'static, Key>>>,
        workload: &crate::tests::workload::Workload,
        prepare_delete: bool,
    ) -> Result<(), Error> {
        let _ = env_logger::builder().is_test(true).try_init();
        let (_, data) = workload.generate_workload_data_as_vec();
        workload.insert_parallel(&data, store.clone()).await?;
        if prepare_delete {
            let _ = store.write().await.put("test_key", "test").await?;
            let _ = store.write().await.delete("test_key").await?;
        }
        Ok(())
    }
    // Generate test to find keys after compaction
    #[tokio::test]
    async fn datastore_gc_test_success() {
        let root = tempdir().unwrap();
        let path = root.path().join("gc_test_1");
        let s_engine = DataStore::open_without_background("test", path.clone())
            .await
            .unwrap();
        let store = Arc::new(RwLock::new(s_engine));
        let workload_size = 5000;
        let key_len = 5;
        let val_len = 5;
        let write_read_ratio = 0.5;
        let workload =
            crate::tests::workload::Workload::new(workload_size, key_len, val_len, write_read_ratio);
        if let Err(err) = setup(store.clone(), &workload, true).await {
            log::error!("Setup failed {}", err);
            return;
        }
        let storage_reader = store.read().await;
        let config = storage_reader.gc.config.clone();
        #[allow(unused_variables)] // for non linux based envinronment
        let res = GC::gc_handler(
            &config,
            Arc::clone(&storage_reader.gc_table),
            Arc::clone(&storage_reader.gc_log),
            Arc::clone(&storage_reader.key_range),
            Arc::clone(&storage_reader.read_only_memtables),
            Arc::clone(&storage_reader.gc_updated_entries),
            Arc::clone(&storage_reader.gc.punch_marker),
        )
        .await;

        #[cfg(target_os = "linux")]
        {
            assert!(res.is_ok())
        }
    }

    #[tokio::test]
    async fn datastore_gc_test_unsupported_platform() {
        let root = tempdir().unwrap();
        let path = root.path().join("gc_test_2");
        let s_engine = DataStore::open_without_background("test", path.clone())
            .await
            .unwrap();
        let store = Arc::new(RwLock::new(s_engine));
        let workload_size = 5000;
        let key_len = 5;
        let val_len = 5;
        let write_read_ratio = 0.5;
        let workload =
            crate::tests::workload::Workload::new(workload_size, key_len, val_len, write_read_ratio);
        if let Err(err) = setup(store.clone(), &workload, true).await {
            log::error!("Setup failed {}", err);
            return;
        }
        let storage_reader = store.read().await;
        let config = storage_reader.gc.config.clone();
        let _res = GC::gc_handler(
            &config,
            Arc::clone(&storage_reader.gc_table),
            Arc::clone(&storage_reader.gc_log),
            Arc::clone(&storage_reader.key_range),
            Arc::clone(&storage_reader.read_only_memtables),
            Arc::clone(&storage_reader.gc_updated_entries),
            Arc::clone(&storage_reader.gc.punch_marker),
        )
        .await;

        #[cfg(not(target_os = "linux"))]
        {
            assert!(_res.is_ok());
        }
    }

    #[tokio::test]
    async fn datastore_gc_test_tail_shifted() {
        let root = tempdir().unwrap();
        let path = root.path().join("gc_test_3");
        let s_engine = DataStore::open_without_background("test", path.clone())
            .await
            .unwrap();
        let store = Arc::new(RwLock::new(s_engine));
        let workload_size = 5000;
        let key_len = 5;
        let val_len = 5;
        let write_read_ratio = 0.5;
        let workload =
            crate::tests::workload::Workload::new(workload_size, key_len, val_len, write_read_ratio);
        if let Err(err) = setup(store.clone(), &workload, true).await {
            log::error!("Setup failed {}", err);
            return;
        }

        let storage_reader = store.read().await;
        let config = storage_reader.gc.config.clone();
        let initial_tail_offset = storage_reader.gc_log.read().await.tail_offset;

        let _ = GC::gc_handler(
            &config,
            Arc::clone(&storage_reader.gc_table),
            Arc::clone(&storage_reader.gc_log),
            Arc::clone(&storage_reader.key_range),
            Arc::clone(&storage_reader.read_only_memtables),
            Arc::clone(&storage_reader.gc_updated_entries),
            Arc::clone(&storage_reader.gc.punch_marker),
        )
        .await;
        drop(storage_reader);
        // call a put operation to sync gc with memtable
        let _ = store.write().await.put("test_key", "test_val").await;
        assert!(store.read().await.gc.vlog.read().await.tail_offset >= initial_tail_offset);
    }

    #[tokio::test]
    async fn datastore_gc_test_free_before_synchronization() {
        let root = tempdir().unwrap();
        let path = root.path().join("gc_test_free");
        let s_engine = DataStore::open_without_background("test", path.clone())
            .await
            .unwrap();
        let store = Arc::new(RwLock::new(s_engine));
        let workload_size = 5000;
        let key_len = 5;
        let val_len = 5;
        let write_read_ratio = 0.5;
        let workload =
            crate::tests::workload::Workload::new(workload_size, key_len, val_len, write_read_ratio);
        if let Err(err) = setup(store.clone(), &workload, true).await {
            log::error!("Setup failed {}", err);
            return;
        }
        let storage_reader = store.read().await;
        let config = storage_reader.gc.config.clone();
        let initial_tail_offset = storage_reader.gc_log.read().await.tail_offset;

        let _ = GC::gc_handler(
            &config,
            Arc::clone(&storage_reader.gc_table),
            Arc::clone(&storage_reader.gc_log),
            Arc::clone(&storage_reader.key_range),
            Arc::clone(&storage_reader.read_only_memtables),
            Arc::clone(&storage_reader.gc_updated_entries),
            Arc::clone(&storage_reader.gc.punch_marker),
        )
        .await;
        drop(storage_reader);
        // no tail should happen because we have not synchronize gc entries with store memtable±±
        assert!(store.read().await.gc.vlog.read().await.tail_offset == initial_tail_offset);
    }

    #[tokio::test]
    async fn datastore_gc_test_tail_shifted_to_correct_position() {
        let bytes_to_scan_for_garbage_colection = SizeUnit::Bytes.as_bytes(100);
        let root = tempdir().unwrap();
        let path = root.path().join("gc_test_4");
        let s_engine = DataStore::open_without_background("test", path.clone())
            .await
            .unwrap();
        let store = Arc::new(RwLock::new(s_engine));
        let workload_size = 5;
        let key_len = 5;
        let val_len = 5;
        let write_read_ratio = 0.5;
        let workload =
            crate::tests::workload::Workload::new(workload_size, key_len, val_len, write_read_ratio);
        if let Err(err) = setup(store.clone(), &workload, true).await {
            log::error!("Setup failed {}", err);
            return;
        }
        let string_length = 5;
        let vaue_len = 3;
        let storage_reader = store.read().await;
        let mut config = storage_reader.gc.config.clone();

        let initial_tail_offset = storage_reader.gc_log.read().await.tail_offset;
        config.gc_chunk_size = bytes_to_scan_for_garbage_colection;
        let _ = GC::gc_handler(
            &config,
            Arc::clone(&storage_reader.gc_table),
            Arc::clone(&storage_reader.gc_log),
            Arc::clone(&storage_reader.key_range),
            Arc::clone(&storage_reader.read_only_memtables),
            Arc::clone(&storage_reader.gc_updated_entries),
            Arc::clone(&storage_reader.gc.punch_marker),
        )
        .await;
        drop(storage_reader);
        // call a put operation to sync gc with memtable
        let _ = store.write().await.put("test_key", "test_val").await;
        let max_extention_length = SIZE_OF_U32   // Key Size(for fetching key length)
            +SIZE_OF_U32            // Value Length(for fetching value length)
            + SIZE_OF_U64           // Date Length
            + SIZE_OF_U8            // Tombstone marker len
            + string_length         // Key Len
            + vaue_len; // Value Len
        assert!(
            store.read().await.gc.vlog.read().await.tail_offset
                <= initial_tail_offset + bytes_to_scan_for_garbage_colection + max_extention_length
        );
    }

    #[tokio::test]
    async fn datastore_gc_test_head_shifted() {
        let bytes_to_scan_for_garbage_colection = SizeUnit::Bytes.as_bytes(100);
        let root = tempdir().unwrap();
        let path = root.path().join("gc_test_5");
        let s_engine = DataStore::open_without_background("test", path.clone())
            .await
            .unwrap();
        let store = Arc::new(RwLock::new(s_engine));
        let _ = store.write().await.put("test_key", "test_val").await;
        let _ = store.write().await.delete("test_key").await;
        let workload_size = 2;
        let key_len = 5;
        let val_len = 5;
        let write_read_ratio = 0.5;
        let workload =
            crate::tests::workload::Workload::new(workload_size, key_len, val_len, write_read_ratio);
        if let Err(err) = setup(store.clone(), &workload, true).await {
            log::error!("Setup failed {}", err);
            return;
        }
        (store.write().await).gc.config.gc_chunk_size = bytes_to_scan_for_garbage_colection;
        let storage_reader = store.read().await;
        let initial_head_offset = storage_reader.gc_log.read().await.head_offset;
        let _ = GC::gc_handler(
            &storage_reader.gc.config.clone(),
            Arc::clone(&storage_reader.gc_table),
            Arc::clone(&storage_reader.gc_log),
            Arc::clone(&storage_reader.key_range),
            Arc::clone(&storage_reader.read_only_memtables),
            Arc::clone(&storage_reader.gc_updated_entries),
            Arc::clone(&storage_reader.gc.punch_marker),
        )
        .await;
        drop(storage_reader);
        // call a put operation to sync gc with memtable
        let _ = store.write().await.put("test_key", "test_val").await;
        assert!(store.read().await.gc.vlog.read().await.head_offset != initial_head_offset);
    }

    #[tokio::test]
    async fn datastore_gc_test_no_entry_to_collect() {
        let prepare_delete = false;
        let root = tempdir().unwrap();
        let path = root.path().join("gc_test_no_delete");
        let s_engine = DataStore::open_without_background("test", path.clone())
            .await
            .unwrap();
        let store = Arc::new(RwLock::new(s_engine));
        let workload_size = 5000;
        let key_len = 5;
        let val_len = 5;
        let write_read_ratio = 0.5;
        let workload =
            crate::tests::workload::Workload::new(workload_size, key_len, val_len, write_read_ratio);
        if let Err(err) = setup(store.clone(), &workload, prepare_delete).await {
            log::error!("Setup failed {}", err);
            return;
        }
        let storage_reader = store.read().await;
        let config = storage_reader.gc.config.clone();
        let initial_tail_offset = storage_reader.gc_log.read().await.tail_offset;

        let _ = GC::gc_handler(
            &config,
            Arc::clone(&storage_reader.gc_table),
            Arc::clone(&storage_reader.gc_log),
            Arc::clone(&storage_reader.key_range),
            Arc::clone(&storage_reader.read_only_memtables),
            Arc::clone(&storage_reader.gc_updated_entries),
            Arc::clone(&storage_reader.gc.punch_marker),
        )
        .await;
        drop(storage_reader);
        // no tail should happen because no entries to collect
        assert!(store.read().await.gc.vlog.read().await.tail_offset == initial_tail_offset);
    }

    #[tokio::test]

    async fn datastore_gc_test_punch_hole() {
        #[cfg(target_os = "linux")]
        {
            use std::io::{Read, Seek, SeekFrom, Write};
            use tempfile::NamedTempFile;

            const PUNCH_START: i64 = 0;
            const PUNCH_LENGTH: usize = 7;
            let mut temp_file = NamedTempFile::new().unwrap();
            let file_path = temp_file.path().to_path_buf();
            writeln!(temp_file, "Sample1Sample2Sample3Sample4Sample5Sample6").unwrap();

            temp_file.flush().unwrap();
            temp_file.as_file_mut().seek(SeekFrom::Start(0)).unwrap();

            // Before punch, offsets within this range should be contain bytes
            let mut buffer = [0; PUNCH_LENGTH];
            let bytes_read = temp_file.read(&mut buffer).unwrap();
            assert_eq!(bytes_read, PUNCH_LENGTH);
            assert_eq!(&buffer, b"Sample1"); // bytes present in offset

            let punch_res = GC::punch_holes(file_path, PUNCH_START, PUNCH_LENGTH as i64).await;

            assert!(punch_res.is_ok());

            let inner_file = temp_file.as_file_mut();
            inner_file.seek(SeekFrom::Start(0)).unwrap();

            // After punch, offsets within this range should be zero
            let mut buffer = [0; PUNCH_LENGTH];
            let bytes_read = inner_file.read(&mut buffer).unwrap();
            assert_eq!(bytes_read, PUNCH_LENGTH);
            assert_eq!(&buffer, &[0; 7]); // all set to zero
        }
    }
}
