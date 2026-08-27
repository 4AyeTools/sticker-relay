use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

use crate::models::{FeishuSendState, StickerRecord};

const MAX_STICKER_BYTES: usize = 10 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
struct StickerManifest {
    version: u8,
    stickers: Vec<StickerRecord>,
}

#[derive(Debug)]
pub struct AddResult {
    pub added: bool,
    pub record: Option<StickerRecord>,
}

#[derive(Debug)]
pub struct StickerMigrationResult {
    pub previous_root: PathBuf,
    pub previous_records: Vec<StickerRecord>,
    pub migrated_count: usize,
}

pub struct StickerStore {
    root_directory: PathBuf,
    manifest_path: PathBuf,
    sticker_directory: PathBuf,
    records: Vec<StickerRecord>,
}

impl StickerStore {
    pub fn new(root_directory: PathBuf) -> Self {
        let root_directory = absolute_path(root_directory);
        Self {
            manifest_path: root_directory.join("manifest.json"),
            sticker_directory: root_directory.join("stickers"),
            root_directory,
            records: Vec::new(),
        }
    }

    pub fn initialize(&mut self) -> Result<()> {
        fs::create_dir_all(&self.sticker_directory)?;
        self.records = read_manifest(&self.manifest_path);
        self.prune_missing_files()
    }

    pub fn list(&self) -> Vec<StickerRecord> {
        let mut records = self.records.clone();
        records.sort_by_key(|record| Reverse(record.imported_at));
        records
    }

    pub fn get(&self, id: &str) -> Option<&StickerRecord> {
        self.records.iter().find(|record| record.id == id)
    }

    pub fn get_file_path(&self, id: &str) -> Option<PathBuf> {
        self.get(id)
            .map(|record| self.sticker_directory.join(&record.filename))
    }

    pub fn get_data_url(&self, id: &str) -> Option<String> {
        let record = self.get(id)?;
        let bytes = fs::read(self.sticker_directory.join(&record.filename)).ok()?;
        Some(format!(
            "data:{};base64,{}",
            record.mime_type,
            STANDARD.encode(bytes)
        ))
    }

    pub fn add_buffer(
        &mut self,
        buffer: &[u8],
        mime_type: Option<&str>,
        wechat_md5: Option<String>,
        source_url: Option<String>,
    ) -> Result<AddResult> {
        if buffer.is_empty() || buffer.len() > MAX_STICKER_BYTES {
            return Ok(AddResult {
                added: false,
                record: None,
            });
        }

        let detected = sniff_image_mime(buffer).map(str::to_owned).or_else(|| {
            mime_type
                .filter(|value| value.starts_with("image/"))
                .map(str::to_owned)
        });
        let Some(mime_type) = detected else {
            return Ok(AddResult {
                added: false,
                record: None,
            });
        };

        let sha256 = hex::encode(Sha256::digest(buffer));
        if let Some(existing) = self.records.iter().find(|record| record.sha256 == sha256) {
            return Ok(AddResult {
                added: false,
                record: Some(existing.clone()),
            });
        }

        let filename = format!("{}{}", sha256, extension_for_mime(&mime_type));
        let record = StickerRecord {
            id: sha256.chars().take(24).collect(),
            sha256,
            wechat_md5,
            filename: filename.clone(),
            mime_type,
            bytes: buffer.len() as u64,
            source_url,
            imported_at: now_millis(),
            feishu_state: FeishuSendState::Pending,
            feishu_message_id: None,
            error_message: None,
        };

        fs::create_dir_all(&self.sticker_directory)?;
        let destination = self.sticker_directory.join(filename);
        if !destination.exists() {
            fs::write(&destination, buffer)?;
        }
        self.records.insert(0, record.clone());
        self.save()?;
        Ok(AddResult {
            added: true,
            record: Some(record),
        })
    }

    pub fn mark_sending(&mut self, id: &str) -> Result<()> {
        self.patch(id, FeishuSendState::Sending, None, None)
    }

    pub fn mark_sent(&mut self, id: &str, message_id: Option<String>) -> Result<()> {
        self.patch(id, FeishuSendState::Sent, message_id, None)
    }

    pub fn mark_failed(&mut self, id: &str, message: String) -> Result<()> {
        let message: String = message.chars().take(500).collect();
        self.patch(id, FeishuSendState::Failed, None, Some(message))
    }

    pub fn root_directory(&self) -> &Path {
        &self.root_directory
    }

    pub fn migrate_to(&mut self, destination_root: PathBuf) -> Result<StickerMigrationResult> {
        if !destination_root.is_absolute() {
            return Err(anyhow!("新的表情库目录必须是绝对路径"));
        }
        let destination = absolute_path(destination_root);
        let previous_root = self.root_directory.clone();
        let previous_records = self.records.clone();
        if same_path(&destination, &previous_root) {
            return Ok(StickerMigrationResult {
                previous_root,
                previous_records,
                migrated_count: 0,
            });
        }
        if paths_overlap(&previous_root, &destination) {
            return Err(anyhow!(
                "新旧表情库目录不能互相包含，请选择另一个独立文件夹"
            ));
        }

        let destination_stickers = destination.join("stickers");
        let destination_manifest = destination.join("manifest.json");
        fs::create_dir_all(&destination_stickers)?;
        let destination_records = read_manifest(&destination_manifest);

        for record in &previous_records {
            let source = self.sticker_directory.join(&record.filename);
            let target = destination_stickers.join(&record.filename);
            if target.exists() {
                let existing = fs::read(&target)?;
                if hex::encode(Sha256::digest(existing)) != record.sha256 {
                    return Err(anyhow!(
                        "目标目录存在同名但内容不同的文件：{}",
                        record.filename
                    ));
                }
            } else {
                fs::copy(&source, &target)
                    .with_context(|| format!("复制表情失败：{}", record.filename))?;
            }
        }

        let mut merged: HashMap<String, StickerRecord> = destination_records
            .into_iter()
            .map(|record| (record.sha256.clone(), record))
            .collect();
        for record in &previous_records {
            merged.insert(record.sha256.clone(), record.clone());
        }
        let merged_records: Vec<StickerRecord> = merged.into_values().collect();
        write_manifest(&destination_manifest, &merged_records)?;

        self.set_root_directory(destination);
        self.records = merged_records;
        self.prune_missing_files()?;
        Ok(StickerMigrationResult {
            previous_root,
            migrated_count: previous_records.len(),
            previous_records,
        })
    }

    pub fn restore_root(&mut self, root_directory: PathBuf) -> Result<()> {
        self.set_root_directory(root_directory);
        self.initialize()
    }

    pub fn cleanup_previous_library(&self, migration: &StickerMigrationResult) {
        if same_path(&migration.previous_root, &self.root_directory) {
            return;
        }
        let old_sticker_directory = migration.previous_root.join("stickers");
        for record in &migration.previous_records {
            let _ = fs::remove_file(old_sticker_directory.join(&record.filename));
        }
        let _ = fs::remove_file(migration.previous_root.join("manifest.json"));
        let _ = fs::remove_dir(&old_sticker_directory);
        let _ = fs::remove_dir(&migration.previous_root);
    }

    pub fn delete_ids(&mut self, ids: &[String]) -> Result<Vec<StickerRecord>> {
        let ids: HashSet<&str> = ids.iter().map(String::as_str).collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let removed: Vec<StickerRecord> = self
            .records
            .iter()
            .filter(|record| ids.contains(record.id.as_str()))
            .cloned()
            .collect();
        if removed
            .iter()
            .any(|record| record.feishu_state == FeishuSendState::Sending)
        {
            return Err(anyhow!("有表情正在发送到飞书，请等待发送完成后再删除"));
        }
        if removed.is_empty() {
            return Ok(removed);
        }

        let previous_records = self.records.clone();
        self.records
            .retain(|record| !ids.contains(record.id.as_str()));
        if let Err(error) = self.save() {
            self.records = previous_records;
            return Err(error);
        }
        for record in &removed {
            let path = self.sticker_directory.join(&record.filename);
            if let Err(error) = fs::remove_file(&path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    // The manifest is already authoritative. A locked orphan file can be
                    // cleaned manually without making the deleted sticker reappear.
                    continue;
                }
            }
        }
        Ok(removed)
    }

    pub fn export_zip(&self, destination: &Path) -> Result<usize> {
        if self.records.is_empty() {
            return Err(anyhow!("当前没有可导出的表情"));
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = fs::File::create(destination)?;
        let mut archive = ZipWriter::new(file);
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .compression_level(Some(9));

        for record in self.list() {
            let path = self.sticker_directory.join(&record.filename);
            if !path.exists() {
                continue;
            }
            archive.start_file(format!("stickers/{}", record.filename), options)?;
            let mut source = fs::File::open(path)?;
            let mut buffer = Vec::new();
            source.read_to_end(&mut buffer)?;
            archive.write_all(&buffer)?;
        }
        archive.start_file("manifest.json", options)?;
        let payload = serde_json::to_vec_pretty(&StickerManifest {
            version: 1,
            stickers: self.list(),
        })?;
        archive.write_all(&payload)?;
        archive.write_all(b"\n")?;
        archive.finish()?;
        Ok(self.records.len())
    }

    fn patch(
        &mut self,
        id: &str,
        state: FeishuSendState,
        message_id: Option<String>,
        error_message: Option<String>,
    ) -> Result<()> {
        if let Some(record) = self.records.iter_mut().find(|record| record.id == id) {
            record.feishu_state = state;
            if message_id.is_some() || record.feishu_state == FeishuSendState::Sent {
                record.feishu_message_id = message_id;
            }
            record.error_message = error_message;
            self.save()?;
        }
        Ok(())
    }

    fn prune_missing_files(&mut self) -> Result<()> {
        let before = self.records.len();
        self.records.retain(|record| {
            fs::metadata(self.sticker_directory.join(&record.filename))
                .map(|metadata| metadata.is_file() && metadata.len() > 0)
                .unwrap_or(false)
        });
        if before != self.records.len() {
            self.save()?;
        }
        Ok(())
    }

    fn save(&self) -> Result<()> {
        write_manifest(&self.manifest_path, &self.records)
    }

    fn set_root_directory(&mut self, root_directory: PathBuf) {
        self.root_directory = absolute_path(root_directory);
        self.manifest_path = self.root_directory.join("manifest.json");
        self.sticker_directory = self.root_directory.join("stickers");
    }
}

pub fn sniff_image_mime(buffer: &[u8]) -> Option<&'static str> {
    if buffer.starts_with(&[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]) {
        return Some("image/png");
    }
    if buffer.starts_with(b"GIF87a") || buffer.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if buffer.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    if buffer.len() >= 12 && &buffer[0..4] == b"RIFF" && &buffer[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

fn extension_for_mime(mime_type: &str) -> &'static str {
    match mime_type {
        "image/gif" => ".gif",
        "image/png" => ".png",
        "image/jpeg" => ".jpg",
        "image/webp" => ".webp",
        _ => ".bin",
    }
}

fn read_manifest(path: &Path) -> Vec<StickerRecord> {
    fs::read(path)
        .ok()
        .and_then(|raw| serde_json::from_slice::<StickerManifest>(&raw).ok())
        .map(|manifest| manifest.stickers)
        .unwrap_or_default()
}

fn write_manifest(path: &Path, records: &[StickerRecord]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    let payload = StickerManifest {
        version: 1,
        stickers: records.to_vec(),
    };
    let mut bytes = serde_json::to_vec_pretty(&payload)?;
    bytes.push(b'\n');
    fs::write(&temporary, bytes)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temporary, path)?;
    Ok(())
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn absolute_path(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    }
}

fn normalized_for_compare(path: &Path) -> String {
    #[cfg(windows)]
    {
        path.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    }
    #[cfg(not(windows))]
    {
        path.to_string_lossy().trim_end_matches('/').to_string()
    }
}

fn same_path(first: &Path, second: &Path) -> bool {
    normalized_for_compare(first) == normalized_for_compare(second)
}

fn paths_overlap(first: &Path, second: &Path) -> bool {
    let first = normalized_for_compare(first);
    let second = normalized_for_compare(second);
    #[cfg(windows)]
    {
        first.starts_with(&(second.clone() + "\\")) || second.starts_with(&(first + "\\"))
    }
    #[cfg(not(windows))]
    {
        first.starts_with(&(second.clone() + "/")) || second.starts_with(&(first + "/"))
    }
}

#[cfg(test)]
mod tests {
    use super::{now_millis, sniff_image_mime, StickerStore};

    #[test]
    fn detects_supported_images() {
        assert_eq!(sniff_image_mime(b"GIF89a"), Some("image/gif"));
        assert_eq!(
            sniff_image_mime(&[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
            Some("image/png")
        );
        assert_eq!(sniff_image_mime(&[0xff, 0xd8, 0xff]), Some("image/jpeg"));
    }

    #[test]
    fn deletes_sticker_record_and_file() {
        let root = std::env::temp_dir().join(format!(
            "wechat-feishu-sticker-delete-test-{}-{}",
            std::process::id(),
            now_millis()
        ));
        let mut store = StickerStore::new(root.clone());
        store.initialize().expect("initialize store");
        let record = store
            .add_buffer(
                &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 1],
                Some("image/png"),
                Some("test-md5".to_string()),
                None,
            )
            .expect("add sticker")
            .record
            .expect("record");
        let file_path = store.get_file_path(&record.id).expect("file path");
        assert!(file_path.exists());

        let removed = store
            .delete_ids(std::slice::from_ref(&record.id))
            .expect("delete sticker");
        assert_eq!(removed.len(), 1);
        assert!(store.list().is_empty());
        assert!(!file_path.exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
