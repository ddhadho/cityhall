use cityhall::{Result, StorageEngine, Wal};
use std::thread;
use std::time::Duration;
use std::time::SystemTime;
use tempfile::tempdir;
use tempfile::TempDir;
use std::sync::Arc;
use parking_lot::RwLock;

#[test]
fn test_basic_put_and_get() -> Result<()> {
    let dir = tempdir()?;
    let path = dir.path().to_path_buf();
    let wal_path = path.join("test.wal");
    let wal = Wal::new(&wal_path, 1024)?;
    let wal = Arc::new(RwLock::new(wal));
    let mut engine = StorageEngine::new(path, 1024 * 1024, wal)?;
    
    engine.put(b"key1".to_vec(), b"value1".to_vec())?;
    engine.put(b"key2".to_vec(), b"value2".to_vec())?;
    engine.put(b"key3".to_vec(), b"value3".to_vec())?;
    
    assert_eq!(engine.get(b"key1")?, Some(b"value1".to_vec()));
    assert_eq!(engine.get(b"key2")?, Some(b"value2".to_vec()));
    assert_eq!(engine.get(b"key3")?, Some(b"value3".to_vec()));
    assert_eq!(engine.get(b"nonexistent")?, None);
    
    Ok(())
}
#[test]
fn test_overwrite() -> Result<()> {
    let dir = tempdir()?;
    let path = dir.path().to_path_buf();
    let wal_path = path.join("test.wal");
    let wal = Wal::new(&wal_path, 1024)?;
    let wal = Arc::new(RwLock::new(wal));
    let mut engine = StorageEngine::new(path, 1024 * 1024, wal)?;

    engine.put(b"key".to_vec(), b"value1".to_vec())?;
    assert_eq!(engine.get(b"key")?, Some(b"value1".to_vec()));

    engine.put(b"key".to_vec(), b"value2".to_vec())?;
    assert_eq!(engine.get(b"key")?, Some(b"value2".to_vec()));

    Ok(())
}

#[test]
fn test_crash_recovery() -> Result<()> {
    let dir = tempdir()?;
    let dir_path = dir.path().to_path_buf();

    {    
        let wal_path = dir_path.join("test.wal");
        let wal = Wal::new(&wal_path, 1024)?;
        let wal = Arc::new(RwLock::new(wal));
        let mut engine = StorageEngine::new(dir_path.clone(), 1024 * 1024, wal)?;

        engine.put(b"key1".to_vec(), b"value1".to_vec())?;
        engine.put(b"key2".to_vec(), b"value2".to_vec())?;
        engine.put(b"key3".to_vec(), b"value3".to_vec())?;
    }

    {
        let wal_path = dir_path.join("test.wal");
        let wal = Wal::new(&wal_path, 1024)?;
        let wal = Arc::new(RwLock::new(wal));
        let mut engine = StorageEngine::new(dir_path.clone(), 1024 * 1024, wal)?;        

        assert_eq!(engine.get(b"key1")?, Some(b"value1".to_vec()));
        assert_eq!(engine.get(b"key2")?, Some(b"value2".to_vec()));
        assert_eq!(engine.get(b"key3")?, Some(b"value3".to_vec()));
    }

    Ok(())
}

#[test]
fn test_large_values() -> Result<()> {
    let dir = tempdir()?;
    let path = dir.path().to_path_buf();
    let wal_path = path.join("test.wal");
    let wal = Wal::new(&wal_path, 1024)?;
    let wal = Arc::new(RwLock::new(wal));
    let mut engine = StorageEngine::new(path, 1024 * 1024, wal)?;

    let large_value = vec![42u8; 32 * 1024];
    engine.put(b"large1".to_vec(), large_value.clone())?;
    engine.put(b"large2".to_vec(), large_value.clone())?;

    assert_eq!(engine.get(b"large1")?, Some(large_value.clone()));
    assert_eq!(engine.get(b"large2")?, Some(large_value));

    Ok(())
}

#[test]
fn test_many_small_writes() -> Result<()> {
    let dir = tempdir()?;
    let path = dir.path().to_path_buf();
    let wal_path = path.join("test.wal");
    let wal = Wal::new(&wal_path, 1024)?;
    let wal = Arc::new(RwLock::new(wal));
    let mut engine = StorageEngine::new(path, 1024 * 1024, wal)?;

    for i in 0..1000 {
        let key = format!("key_{:04}", i);
        let value = format!("value_{:04}", i);
        engine.put(key.into_bytes(), value.into_bytes())?;
    }

    for i in 0..1000 {
        let key = format!("key_{:04}", i);
        let expected_value = format!("value_{:04}", i);
        assert_eq!(
            engine.get(key.as_bytes())?,
            Some(expected_value.into_bytes())
        );
    }

    Ok(())
}

#[test]
fn test_memtable_flush() -> Result<()> {
    let dir = tempdir()?;
    let path = dir.path().to_path_buf();
    let wal_path = path.join("test.wal");
    let wal = Wal::new(&wal_path, 1024)?;
    let wal = Arc::new(RwLock::new(wal));
    let mut engine = StorageEngine::new(path, 10 * 1024, wal)?;

    for i in 0..1000 {
        let key = format!("key_{:04}", i);
        let value = vec![0u8; 100];
        engine.put(key.into_bytes(), value)?;
    }

    assert!(engine.get(b"key_0999")?.is_some());
    assert!(engine.get(b"key_0000")?.is_some());

    Ok(())
}

#[test]
fn test_empty_database() -> Result<()> {    
    let dir = tempdir()?;
    let path = dir.path().to_path_buf();
    let wal_path = path.join("test.wal");
    let wal = Wal::new(&wal_path, 1024)?;
    let wal = Arc::new(RwLock::new(wal));
    let mut engine = StorageEngine::new(path, 1024 * 1024, wal)?;

    assert_eq!(engine.get(b"any_key")?, None);
    Ok(())
}

#[test]
fn test_binary_keys_and_values() -> Result<()> {
    
    let dir = tempdir()?;
    let path = dir.path().to_path_buf();
    let wal_path = path.join("test.wal");
    let wal = Wal::new(&wal_path, 1024)?;
    let wal = Arc::new(RwLock::new(wal));
    let mut engine = StorageEngine::new(path, 1024 * 1024, wal)?;

    let binary_key = vec![0xFF, 0xFE, 0xFD, 0xFC];
    let binary_value = vec![0x00, 0x01, 0x02, 0x03, 0x04];

    engine.put(binary_key.clone(), binary_value.clone())?;
    assert_eq!(engine.get(&binary_key)?, Some(binary_value));

    Ok(())
}

#[test]
fn test_sequential_time_series_workload() -> Result<()> {

    let dir = tempdir()?;
    let path = dir.path().to_path_buf();
    let wal_path = path.join("test.wal");
    let wal = Wal::new(&wal_path, 1024)?;
    let wal = Arc::new(RwLock::new(wal));
    let mut engine = StorageEngine::new(path, 1024 * 1024, wal)?;

    let base_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_secs();

    for i in 0..100 {
        let timestamp = base_time + i;
        let key = format!("sensor_123_{}", timestamp);
        let value = format!("{{\"temp\": 25.{}, \"humidity\": 60}}", i);
        engine.put(key.into_bytes(), value.into_bytes())?;
    }

    for i in 0..100 {
        let timestamp = base_time + i;
        let key = format!("sensor_123_{}", timestamp);
        assert!(engine.get(key.as_bytes())?.is_some());
    }

    Ok(())
}

#[test]
fn test_wal_memtable_integration() -> Result<()> {
    let dir = tempdir()?;
    let dir_path = dir.path().to_path_buf();
    
    {
        let wal_path = dir_path.join("test.wal");
        let wal = Wal::new(&wal_path, 1024)?;
        let wal = Arc::new(RwLock::new(wal));
        let mut engine = StorageEngine::new(dir_path.clone(), 1024 * 1024, wal)?;
        
        for i in 0..10 {
            let key = format!("metric_{}", i);
            let value = format!("value_{}", i);
            engine.put(key.into_bytes(), value.into_bytes())?;
        }
        for i in 0..10 {
            let key = format!("metric_{}", i);
            let expected = format!("value_{}", i);
            assert_eq!(engine.get(key.as_bytes())?, Some(expected.into_bytes()));
        }
    }
    
    {
        let wal_path = dir_path.join("test.wal");
        let wal = Wal::new(&wal_path, 1024)?;
        let wal = Arc::new(RwLock::new(wal));
        let mut engine = StorageEngine::new(dir_path.clone(), 1024 * 1024, wal)?;
        
        for i in 0..10 {
            let key = format!("metric_{}", i);
            let expected = format!("value_{}", i);
            assert_eq!(engine.get(key.as_bytes())?, Some(expected.into_bytes()));
        }
    }
    
    Ok(())
}

#[test]
fn test_compaction_reduces_sstables() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let path = temp_dir.path().to_path_buf();
    let wal_path = path.join("test.wal");
    let wal = Wal::new(&wal_path, 1024)?;
    let wal = Arc::new(RwLock::new(wal));
    let mut engine = StorageEngine::new(path, 200, wal)?;

    for i in 0..100 {
        let key = format!("key{:04}", i);
        let value = format!("value{}", i);
        engine.put(key.into_bytes(), value.into_bytes())?;
    }
    thread::sleep(Duration::from_millis(200));
    engine.check_and_compact()?;

    for i in 0..50 {
        let key = format!("key{:04}", i);
        let value = format!("value_updated_{}", i);
        engine.put(key.into_bytes(), value.into_bytes())?;
    }
    thread::sleep(Duration::from_millis(200));
    engine.check_and_compact()?;

    engine.force_compact()?;
    thread::sleep(Duration::from_millis(100));

    for i in 0..50 {
        let key = format!("key{:04}", i);
        let expected = format!("value_updated_{}", i);
        assert_eq!(engine.get(key.as_bytes())?, Some(expected.into_bytes()));
    }
    for i in 50..100 {
        let key = format!("key{:04}", i);
        let expected = format!("value{}", i);
        assert_eq!(engine.get(key.as_bytes())?, Some(expected.into_bytes()));
    }

    Ok(())
}

#[test]
fn test_compaction_removes_duplicates() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let path = temp_dir.path().to_path_buf();
    let wal_path = path.join("test.wal");
    let wal = Wal::new(&wal_path, 1024)?;
    let wal = Arc::new(RwLock::new(wal));
    let mut engine = StorageEngine::new(path, 200, wal)?;

    for i in 0..10 {
        engine.put(b"test".to_vec(), format!("version{}", i).into_bytes())?;
        thread::sleep(Duration::from_millis(10));
    }
    thread::sleep(Duration::from_millis(200));
    engine.check_and_compact()?;

    let stats_before = engine.stats();
    let value = engine.get(b"test")?.expect("Key should exist");
    assert_eq!(value, b"version9");

    if stats_before.num_sstables >= 4 {
        engine.force_compact()?;
        thread::sleep(Duration::from_millis(100));
        let value = engine.get(b"test")?.expect("Key should still exist");
        assert_eq!(value, b"version9");
    }

    Ok(())
}
