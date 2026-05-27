use std::fs::{rename, remove_file, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use chrono::Utc;
use std::borrow::Cow;
use std::time::Duration;
use pcap_file::pcapng::{PcapNgWriter, PcapNgBlock, blocks::enhanced_packet::EnhancedPacketBlock, blocks::interface_description::InterfaceDescriptionBlock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageMode {
    Archive,
    Temporary,
}

pub struct PcapStorageManager {
    pub filepath: PathBuf,
    writer: PcapNgWriter<File>,
    mode: StorageMode,
    is_incomplete: bool,
}

impl PcapStorageManager {
    /// Creates a new PCAPNG archiver
    pub fn new(mode: StorageMode, output_dir: impl AsRef<Path>) -> std::io::Result<Self> {
        let dir = output_dir.as_ref();
        if !dir.exists() {
            std::fs::create_dir_all(dir)?;
        }

        let timestamp_str = Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let prefix = match mode {
            StorageMode::Archive => "archive_",
            StorageMode::Temporary => "temp_",
        };
        let filename = format!("{}{}.pcapng", prefix, timestamp_str);
        let filepath = dir.join(filename);

        let file = File::create(&filepath)?;
        let mut writer = PcapNgWriter::new(file).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

        // DLT 127 = IEEE802_11_RADIOTAP - matches mon0 moniotr-mode captures
        let idb = InterfaceDescriptionBlock {
            linktype: pcap_file::DataLink::IEEE802_11_RADIOTAP,
            snaplen: 65535,
            options: vec![],
        };
        writer.write_block(&idb.into_block()).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

        Ok(Self {
            filepath,
            writer,
            mode,
            is_incomplete: false,
        })
    }

    ///Logs a packet cleanly into the PCAP file.
    pub fn write_packet(&mut self, timestamp_ns: u64, packet_data: &[u8]) -> std::io::Result<()> {
        let block = EnhancedPacketBlock {
            interface_id: 0,
            timestamp: Duration::from_nanos(timestamp_ns),
            original_len: packet_data.len() as u32,
            data: Cow::Borrowed(packet_data),
            options: vec![],
        };

        if let Err(e) = self.writer.write_block(&block.into_block()) {
            self.is_incomplete = true;
            return Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()));
        }
        // flush so concurrent readers (HTTP download, injection) see complete blocks
        if let Err(e) = self.writer.get_mut().flush() {
            self.is_incomplete = true;
            return Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()));
        }
        Ok(())
    }

    /// Flags the stream as corrupted (e.g. queue overflowed / packet dropped)
    pub fn mark_incomplete(&mut self) {
        self.is_incomplete = true;
    }

    /// Delete file if mode was temporary and no packets were dropped.
    pub fn cleanup(&self) -> std::io::Result<()> {
        if self.mode == StorageMode::Temporary && !self.is_incomplete && self.filepath.exists() {
            remove_file(&self.filepath)?;
        }
        Ok(())
    }

    /// Consumes struct, flushes output, renames to "incomplete_" if dropped packets.
    pub fn close(self) -> std::io::Result<PathBuf> {

        let mut final_path = self.filepath.clone();
        
        if self.is_incomplete && self.filepath.exists() {
            let filename = self.filepath.file_name().unwrap_or_default().to_string_lossy();
            if !filename.starts_with("incomplete_") {
                let new_filename = format!("incomplete_{}", filename);
                final_path = self.filepath.with_file_name(&new_filename);
                rename(&self.filepath, &final_path)?;
            }
        }
        Ok(final_path)
    }
}
