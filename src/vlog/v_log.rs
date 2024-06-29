//! # Value Log
//!
//! The Sequential Value Log is a crucial component of the LSM Tree storage engine.
//! It provides durability and atomicity guarantees by logging write operations before they are applied to the main data structure.
//! The sstable only stores the value offsets from this file
//!
//! When a write operation is received, the key-value pair is first appended to the Value Log.
//! In the event of a crash or system failure, the Value Log can be replayed to recover the data modifications and bring the MemTable back to a consistent state.
//!
//! ## Value Log Structure
//!
//! The `ValueLog` structure contains the following field:
//!
//! ```rs
//! struct ValueLog {
//!     content: Arc<RwLock<VLogFileNode>>,
//!     head_offset: usize
//!     tail_offset: usize
//! }
//! ```
//!
//! ### content
//!
//! The `content` field is of type `Arc<Rwlock<VLogFileNode>>`. It represents the VLog file and provides concurrent access and modification through the use of an `Arc` (Atomic Reference Counting) and `RwLock`.
//! We use RwLock to ensure multiple threads can read from the log file while permmitting only one thread to write
//!
//! ### head_offset
//!
//! The `head_offset` field stores the start postion of the last entry inserted into the value log
//!
//! ### tail_offset
//!
//! The `tail_offset` field stores the position  we start reading from either normal reads or during garbage collection
//!
//!
//! ## Log File Structure Diagram
//!
//! The `log_file` is structured as follows:
//!
//! ```text
//! +-------------------+
//! |    Key Size       |   (4 bytes)
//! +-------------------+
//! |   Value Size      |   (4 byte)
//! +-------------------+
//! |      Key          |   (variable)
//! +-------------------+
//! |     Value         |   (variable)
//! +-------------------+
//! |   Created At      |   (8 bytes)
//! |                   |
//! +-------------------+
//! |  Is Tombstone     |   (1 byte)
//! |                   |
//! |                   |
//! +-------------------+
//! |    Key Size       |   (4 bytes)
//! +-------------------+
//! |   Value Size      |   (4 byte)
//! +-------------------+
//! |      Key          |   (variable)
//! +-------------------+
//! |     Value         |   (variable)
//! +-------------------+
//! |   Created At      |   (8 bytes)
//! |                   |
//! +-------------------+
//! |  Is Tombstone     |   (1 byte)
//! |                   |
//! |                   |
//! +-------------------+
//! ```
//!
//! - **Key Size**: A 4-byte field representing the length of the key in bytes.
//! - **Value Size**: A 4-byte field representing the length of the value in bytes.
//! - **Key**: The actual key data, which can vary in size.
//! - **Value**: The actual value data, which can vary in size.
//! - **Created At**: A 8-byte field representing the time of insertion in bytes.
//! - **Is Tombstone**: A 1 byte field representing a boolean of deleted or not deleted entry

use chrono::{DateTime, Utc};

use crate::{
    consts::{SIZE_OF_U32, SIZE_OF_U64, SIZE_OF_U8, VLOG_FILE_NAME},
    err::Error,
    fs::{FileAsync, FileNode, VLogFileNode, VLogFs},
    types::{CreatedAt, IsTombStone, Value},
};
use std::path::{Path, PathBuf};
type TotalBytesRead = usize;

#[derive(Debug, Clone)]
pub struct VFile<F: VLogFs> {
    pub file: F,
    pub path: PathBuf,
}

impl<F: VLogFs> VFile<F> {
    pub fn new<P: AsRef<Path> + Send + Sync>(path: P, file: F) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            file,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ValueLog {
    pub content: VFile<VLogFileNode>,
    pub head_offset: usize,
    pub tail_offset: usize,
    pub size: usize,
}

#[derive(PartialEq, Debug, Clone)]
pub struct ValueLogEntry {
    pub ksize: usize,
    pub vsize: usize,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub created_at: DateTime<Utc>,
    pub is_tombstone: bool,
}

impl ValueLog {
    pub async fn new<P: AsRef<Path> + Send + Sync>(dir: P) -> Result<Self, Error> {
        FileNode::create_dir_all(dir.as_ref()).await?;
        let file_path = dir.as_ref().join(VLOG_FILE_NAME);
        let file = VLogFileNode::new(file_path.to_owned(), crate::fs::FileType::ValueLog)
            .await
            .unwrap();
        Ok(Self {
            head_offset: 0,
            tail_offset: 0,
            content: VFile::new(file_path, file),
            size: 0,
        })
    }

    pub async fn append<T: AsRef<[u8]>>(
        &mut self,
        key: T,
        value: T,
        created_at: CreatedAt,
        is_tombstone: bool,
    ) -> Result<usize, Error> {
        let v_log_entry = ValueLogEntry::new(
            key.as_ref().len(),
            value.as_ref().len(),
            key.as_ref().to_vec(),
            value.as_ref().to_vec(),
            created_at,
            is_tombstone,
        );
        let serialized_data = v_log_entry.serialize();
        // Get the current offset before writing(this will be the offset of the value stored in the memtable)
        let last_offset = self.size;
        let data_file = &self.content;
        let _ = data_file.file.node.write_all(&serialized_data).await;
        self.size += serialized_data.len();
        Ok(last_offset as usize)
    }

    pub async fn get(&self, start_offset: usize) -> Result<Option<(Value, IsTombStone)>, Error> {
        self.content.file.get(start_offset).await
    }

    pub async fn sync_to_disk(&self) -> Result<(), Error> {
        self.content.file.node.sync_all().await
    }

    pub async fn recover(&mut self, start_offset: usize) -> Result<Vec<ValueLogEntry>, Error> {
        self.content.file.recover(start_offset).await
    }

    pub async fn read_chunk_to_garbage_collect(
        &self,
        bytes_to_collect: usize,
    ) -> Result<(Vec<ValueLogEntry>, TotalBytesRead), Error> {
        self.content
            .file
            .read_chunk_to_garbage_collect(bytes_to_collect, self.tail_offset as u64)
            .await
    }

    // CAUTION: This deletes the value log file
    pub async fn clear_all(&mut self) {
        if self.content.file.node.metadata().await.is_ok() {
            if let Err(err) = self.content.file.node.remove_dir_all().await {
                log::error!("{}", err);
            }
        }
        self.tail_offset = 0;
        self.head_offset = 0;
    }

    pub fn set_head(&mut self, head: usize) {
        self.head_offset = head;
    }

    pub fn set_tail(&mut self, tail: usize) {
        self.tail_offset = tail;
    }
}

impl ValueLogEntry {
    pub fn new<T: AsRef<[u8]>>(
        ksize: usize,
        vsize: usize,
        key: T,
        value: T,
        created_at: CreatedAt,
        is_tombstone: bool,
    ) -> Self {
        Self {
            ksize,
            vsize,
            key: key.as_ref().to_vec(),
            value: value.as_ref().to_vec(),
            created_at,
            is_tombstone,
        }
    }

    fn serialize(&self) -> Vec<u8> {
        let entry_len = SIZE_OF_U32 + SIZE_OF_U32 + SIZE_OF_U64 + self.key.len() + self.value.len() + SIZE_OF_U8;

        let mut serialized_data = Vec::with_capacity(entry_len);

        serialized_data.extend_from_slice(&(self.key.len() as u32).to_le_bytes());

        serialized_data.extend_from_slice(&(self.value.len() as u32).to_le_bytes());

        serialized_data.extend_from_slice(&self.created_at.timestamp_millis().to_le_bytes());

        serialized_data.push(self.is_tombstone as u8);

        serialized_data.extend_from_slice(&self.key);

        serialized_data.extend_from_slice(&self.value);

        serialized_data
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_serialized() {}
}
