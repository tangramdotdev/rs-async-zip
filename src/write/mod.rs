// Copyright (c) 2021 Harry [Majored] [hello@majored.pw]
// MIT License (https://github.com/Majored/rs-async-zip/blob/main/LICENSE)

//! A module which supports writing ZIP files.

use crate::error::Result;
use crate::header::{CentralDirectoryHeader, GeneralPurposeFlag, LocalFileHeader, EndOfCentralDirectoryHeader};
use crate::Compression;

use std::io::Cursor;

use async_compression::tokio::write::{BzEncoder, DeflateEncoder, LzmaEncoder, XzEncoder, ZstdEncoder};
use chrono::Utc;
use crc32fast::Hasher;
use tokio::io::{AsyncWrite, AsyncWriteExt};

pub struct EntryOptions {
    pub filename: String,
    pub extra: Vec<u8>,
    pub comment: String,
    pub compression: Compression,
}

struct CentralDirectoryEntry {
    header: CentralDirectoryHeader,
    opts: EntryOptions,
}

pub struct ZipFileWriter<'a, W: AsyncWrite + Unpin> {
    writer: &'a mut W,
    cd_entries: Vec<CentralDirectoryEntry>,
    written: usize,
}

impl<'a, W: AsyncWrite + Unpin> ZipFileWriter<'a, W> {
    pub fn new(writer: &'a mut W) -> Self {
        Self { writer, cd_entries: Vec::new(), written: 0 }
    }

    pub async fn write_entry(&mut self, opts: EntryOptions, raw_data: &[u8]) -> Result<()> {
        let mut _compressed_data: Option<Vec<u8>> = None;
        let compressed_data = match &opts.compression {
            Compression::Stored => raw_data,
            _ => {
                _compressed_data = Some(compress(&opts.compression, raw_data).await);
                _compressed_data.as_ref().unwrap()
            }
        };

        let (mod_time, mod_date) = crate::utils::chrono_to_zip_time(&Utc::now());

        let lf_header = LocalFileHeader {
            compressed_size: compressed_data.len() as u32,
            uncompressed_size: raw_data.len() as u32,
            compression: opts.compression.to_u16(),
            crc: compute_crc(raw_data),
            extra_field_length: opts.extra.len() as u16,
            file_name_length: opts.filename.as_bytes().len() as u16,
            mod_time,
            mod_date,
            version: 0,
            flags: GeneralPurposeFlag { data_descriptor: false, encrypted: false },
        };

        let header = CentralDirectoryHeader {
            v_made_by: 0,
            v_needed: 0,
            compressed_size: lf_header.compressed_size,
            uncompressed_size: lf_header.uncompressed_size,
            compression: lf_header.compression,
            crc: lf_header.crc,
            extra_field_length: lf_header.extra_field_length,
            file_name_length: lf_header.file_name_length,
            file_comment_length: opts.comment.len() as u16,
            mod_time: lf_header.mod_time,
            mod_date: lf_header.mod_date,
            flags: lf_header.flags,
            disk_start: 0,
            inter_attr: 0,
            exter_attr: 0,
            lh_offset: self.written as u32,
        };

        self.written += self.writer.write(&crate::delim::LFHD.to_le_bytes()).await?;
        self.written += self.writer.write(&lf_header.to_slice()).await?;
        self.written += self.writer.write(opts.filename.as_bytes()).await?;
        self.written += self.writer.write(&opts.extra).await?;
        self.written += self.writer.write(compressed_data).await?;

        self.cd_entries.push(CentralDirectoryEntry { header, opts });

        Ok(())
    }

    pub fn stream_write_entry(&mut self, _opts: EntryOptions) -> Result<()> {
        unimplemented!();
    }

    pub async fn close(self) -> Result<()> {
        let cd_offset = self.written;
        let mut cd_size: u32 = 0;

        for entry in &self.cd_entries {
            self.writer.write(&crate::delim::CDFHD.to_le_bytes()).await?;
            self.writer.write(&entry.header.to_slice()).await?;
            self.writer.write(entry.opts.filename.as_bytes()).await?;
            self.writer.write(&entry.opts.extra).await?;
            self.writer.write(entry.opts.comment.as_bytes()).await?;

            cd_size += 4 + 42 + entry.opts.filename.as_bytes().len() as u32;
            cd_size += (entry.opts.extra.len() + entry.opts.comment.len()) as u32;
        }

        let header = EndOfCentralDirectoryHeader {
            disk_num: 0,
            start_cent_dir_disk: 0,
            num_of_entries_disk: self.cd_entries.len() as u16,
            num_of_entries: self.cd_entries.len() as u16,
            size_cent_dir: cd_size,
            cent_dir_offset: cd_offset as u32,
            file_comm_length: 0,
        };

        self.writer.write(&crate::delim::EOCDD.to_le_bytes()).await?;
        self.writer.write(&header.to_slice()).await?;

        Ok(())
    }
}

async fn compress(compression: &Compression, data: &[u8]) -> Vec<u8> {
    // TODO: Reduce reallocations of Vec by making a lower-bound estimate of the length reduction and
    // pre-initialising the Vec to that length. Then truncate() to the actual number of bytes written.
    match compression {
        Compression::Deflate => {
            let mut writer = DeflateEncoder::new(Cursor::new(Vec::new()));
            writer.write_all(data).await.unwrap();
            writer.shutdown().await.unwrap();
            writer.into_inner().into_inner()
        }
        Compression::Bz => {
            let mut writer = BzEncoder::new(Cursor::new(Vec::new()));
            writer.write_all(data).await.unwrap();
            writer.shutdown().await.unwrap();
            writer.into_inner().into_inner()
        }
        Compression::Lzma => {
            let mut writer = LzmaEncoder::new(Cursor::new(Vec::new()));
            writer.write_all(data).await.unwrap();
            writer.shutdown().await.unwrap();
            writer.into_inner().into_inner()
        }
        Compression::Xz => {
            let mut writer = XzEncoder::new(Cursor::new(Vec::new()));
            writer.write_all(data).await.unwrap();
            writer.shutdown().await.unwrap();
            writer.into_inner().into_inner()
        }
        Compression::Zstd => {
            let mut writer = ZstdEncoder::new(Cursor::new(Vec::new()));
            writer.write_all(data).await.unwrap();
            writer.shutdown().await.unwrap();
            writer.into_inner().into_inner()
        }
        _ => unreachable!(),
    }
}

fn compute_crc(data: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(data);
    hasher.finalize()
}
