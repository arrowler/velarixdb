use crate::memtable::InMemoryTable;
use crate::sstable::SSTable;
use std::collections::HashMap;
use std::{fs, io};
use std::{path::PathBuf, sync::Arc};
use crossbeam_skiplist::SkipMap;
use uuid::Uuid;

const BUCKET_LOW: f64 = 0.5;
const BUCKET_HIGH: f64 = 1.5;
const MIN_SSTABLE_SIZE: usize = 32;
const MIN_TRESHOLD: usize = 4;
const MAX_TRESHOLD: usize = 32;
const  BUCKET_DIRECTORY_PREFIX: &str = "bucket";
#[derive(Debug)]
pub struct BucketMap {
    dir: PathBuf,
    buckets: HashMap<Uuid, Bucket>,
}
#[derive(Debug)]
pub(crate) struct Bucket {
    pub(crate) id: Uuid,
    pub(crate) dir: PathBuf,
    pub(crate) avarage_size: usize,
    pub(crate) sstables: Vec<SSTablePath>,
}
#[derive(Debug, Clone)]
pub struct SSTablePath{
    file_path: String,
    hotness: u64
}
impl  SSTablePath{
    pub fn  new(file_path: String)-> Self{
     Self{
       file_path,
       hotness:0
     }
    }
    pub fn increase_hotness(&mut self){
       self.hotness+=1;
    }
    pub fn get_path(&self)-> String{
        self.file_path.clone()
     }

     pub fn get_hotness(&self)-> u64{
        self.hotness
     }
}

pub trait ProvideSizeInBytes {
    fn get_index(&self) -> Arc<SkipMap<Vec<u8>, (usize, u64)>>; // usize for value offset, u64 to store entry creation date in milliseconds
    fn size(&self)-> usize;
}

impl Bucket {
    pub fn new(dir: PathBuf) -> Self {
        let bucket_id = Uuid::new_v4();
        let bucket_dir = dir.join(BUCKET_DIRECTORY_PREFIX.to_string() + bucket_id.to_string().as_str()) ;
        fs::create_dir(bucket_dir.clone()).expect("Unable to create file");
        Self {
            id: bucket_id,
            dir: bucket_dir,
            avarage_size: 0,
            sstables: Vec::new(),
        }
    }
}

impl BucketMap {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            buckets: HashMap::new(),
        }
    }


    pub fn insert_to_appropriate_bucket<T: ProvideSizeInBytes>(&mut self, memtable: &T, hotness: u64) -> io::Result<SSTablePath> {
        let added_to_bucket = false;
            
            for (_, bucket) in &mut self.buckets {
                
                // if (bucket low * bucket avg) is less than sstable size 
                if (bucket.avarage_size as f64 * BUCKET_LOW  < memtable.size() as f64) 
        
                    // and sstable size is less than (bucket avg * bucket high)
                    && (memtable.size() < (bucket.avarage_size as f64 * BUCKET_HIGH) as usize)
                    
                    // or the (sstable size is less than min sstabke size) and (bucket avg is less than the min sstable size )
                    || ((memtable.size() as usize) < MIN_SSTABLE_SIZE && bucket.avarage_size  < MIN_SSTABLE_SIZE)
                {
                    
                    let mut sstable = SSTable::new(bucket.dir.clone(), true);
                    sstable.set_index(memtable.get_index());
                    match sstable.write_to_file() {
                        Ok(_) => {
                            // add sstable to bucket
                            let sstable_path = SSTablePath{
                                file_path: sstable.get_path(),
                                hotness
                            };
                            bucket.sstables.push(sstable_path.clone());
                            bucket.sstables.iter_mut().for_each(|s| s.increase_hotness());
                            bucket.avarage_size = (bucket.sstables.iter().map(|s| fs::metadata(s.get_path()).unwrap().len()).sum::<u64>() / bucket.sstables.len() as u64) as usize;
                            // for test
                            self.buckets.iter().for_each(|b|{
                                b.1.sstables.iter().for_each(|s|{
                                    println!("Inserted to bucket {:?} \n", s);
                                });
                            });
                           
                            return Ok(sstable_path);
                        }
                        Err(err) => {
                            return Err(io::Error::new(err.kind(), err.to_string()));
                        },
                    }
                }
            }
        
            // create a new bucket if none of the condition above was satisfied
            if !added_to_bucket {
                let mut bucket = Bucket::new(self.dir.clone());
                let mut sstable = SSTable::new(bucket.dir.clone(), true);
                    sstable.set_index(memtable.get_index());
                    match sstable.write_to_file() {
                        Ok(_) => {
                            // add sstable to bucket
                            let sstable_path = SSTablePath{
                                file_path: sstable.get_path(),
                                hotness:1
                            };
                            bucket.sstables.push(sstable_path.clone());
                            bucket.avarage_size = fs::metadata(sstable.get_path()).unwrap().len() as usize;
                            self.buckets.insert(bucket.id, bucket);
                            
                            return Ok(sstable_path);
                        }
                        Err(err) => {
                            return Err(io::Error::new(err.kind(), err.to_string()));
                        },
                    }
            }
            
            Err(io::Error::new(io::ErrorKind::Other, "No condition for insertion was stisfied"))
        }



        pub fn extract_buckets_to_compact(&self) -> (Vec<Bucket>, Vec<(Uuid, Vec<SSTablePath>)>) {

            let mut sstables_to_delete: Vec<(Uuid, Vec<SSTablePath>)> = Vec::new();
            let mut buckets_to_compact: Vec<Bucket> = Vec::new();
            // Extract buckets
            self.buckets
                .iter()
                .enumerate()
                .filter(|elem| {
                    // b for bucket :)
                    let b = elem.1 .1;
                    b.sstables.len() >= MIN_TRESHOLD
                })
                .for_each(|(_, elem)| {
                    let b = elem.1;
                    if b.sstables.len() > MAX_TRESHOLD {
                        sstables_to_delete.push((b.id, b.sstables[0..MAX_TRESHOLD].to_vec()));
                    }
                    buckets_to_compact.push(Bucket {
                        sstables: b.sstables[0..MAX_TRESHOLD].to_vec(),
                        id: b.id,
                        dir: b.dir.clone(),
                        avarage_size: b.avarage_size, // passing the average size is redundant here becuase
                                                      // we don't need it for the actual compaction but we leave it to keep things readable
                    })
                });
            (buckets_to_compact, sstables_to_delete)
        }


        // NOTE:  This should be called only after compaction is complete
        pub fn delete_sstables(&mut self, sstables_to_delete: Vec<(Uuid, Vec<SSTablePath>)>) {
            
            // Remove sstables from in memory tables
            for sst in &sstables_to_delete {
                let bucket: &Bucket = self.buckets.get(&sst.0).unwrap();
                let sstables_remaining = &bucket.sstables[0..MAX_TRESHOLD];
                self.buckets.insert(
                    sst.0,
                    Bucket {
                        id: bucket.id,
                        dir: bucket.dir.to_owned(),
                        avarage_size: bucket.avarage_size,
                        sstables: sstables_remaining.to_owned(),
                    },
                );
            }
           // Remove the sstables from bucket 
            for sst in &sstables_to_delete {
                let sst_paths= &sst.1;
                sst_paths.iter().for_each(|sst|{
                    // Attempt to delete the file
                    match fs::remove_file(sst.file_path.to_owned()) {
                        Ok(_) => println!("SS Table deleted successfully."),
                        Err(err) => eprintln!("Error deleting SS Table file: {}", err),
                    }
                })
            }
        }


    
}