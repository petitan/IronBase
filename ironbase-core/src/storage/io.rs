// storage/io.rs
// Low-level I/O operations for storage engine

use std::io::{Read, Write, Seek, SeekFrom};
use crate::error::Result;
use super::StorageEngine;

impl StorageEngine {
    /// Write data to end of file
    /// Returns the offset where data was written
    pub fn write_data(&mut self, data: &[u8]) -> Result<u64> {
        let offset = self.file.seek(SeekFrom::End(0))?;

        // Méret + adat írása
        let len = (data.len() as u32).to_le_bytes();
        self.file.write_all(&len)?;
        self.file.write_all(data)?;

        Ok(offset)
    }

    /// Read data from specified offset
    pub fn read_data(&mut self, offset: u64) -> Result<Vec<u8>> {
        self.file.seek(SeekFrom::Start(offset))?;

        // Méret olvasása
        let mut len_bytes = [0u8; 4];
        self.file.read_exact(&mut len_bytes)?;
        let len = u32::from_le_bytes(len_bytes) as usize;

        // Adat olvasása
        let mut data = vec![0u8; len];
        self.file.read_exact(&mut data)?;

        Ok(data)
    }

    /// Get file length
    pub fn file_len(&self) -> Result<u64> {
        Ok(self.file.metadata()?.len())
    }
}
