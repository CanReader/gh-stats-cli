use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const TTL_SECS: u64 = 10 * 60;
const KEEP_SECS: u64 = 24 * 60 * 60;

pub struct Cache {
    dir: Option<PathBuf>,
    read: bool,
}

impl Cache {
    pub fn new(read: bool) -> Self {
        let cache = Self {
            dir: cache_dir(),
            read,
        };
        cache.prune(SystemTime::now());
        cache
    }

    #[cfg(test)]
    pub fn off() -> Self {
        Self {
            dir: None,
            read: false,
        }
    }

    /// Deletes entries older than a day so the folder doesn't keep growing.
    fn prune(&self, now: SystemTime) {
        let Some(entries) = self.dir.as_ref().and_then(|d| fs::read_dir(d).ok()) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| now.duration_since(m).ok())
                .is_some_and(|age| age.as_secs() > KEEP_SECS);
            if old && path.extension().is_some_and(|e| e == "json") {
                let _ = fs::remove_file(path);
            }
        }
    }

    pub fn get(&self, key: &str, now: u64) -> Option<String> {
        if !self.read {
            return None;
        }
        let text = fs::read_to_string(self.path(key)?).ok()?;
        let (stamp, body) = text.split_once('\n')?;
        let stamp: u64 = stamp.parse().ok()?;
        (now.saturating_sub(stamp) < TTL_SECS).then(|| body.to_string())
    }

    pub fn put(&self, key: &str, body: &str, now: u64) {
        // best effort, a failed write only means a miss next time
        let _ = self.try_put(key, body, now);
    }

    fn try_put(&self, key: &str, body: &str, now: u64) -> io::Result<()> {
        let (Some(dir), Some(path)) = (&self.dir, self.path(key)) else {
            return Ok(());
        };
        fs::create_dir_all(dir)?;
        let mut f = open_private(&path)?;
        write!(f, "{now}\n{body}")
    }

    fn path(&self, key: &str) -> Option<PathBuf> {
        let mut h = DefaultHasher::new();
        key.hash(&mut h);
        Some(self.dir.as_ref()?.join(format!("{:016x}.json", h.finish())))
    }
}

fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("gh-stats"))
}

// responses can include private repo names, keep them readable only by you
#[cfg(unix)]
fn open_private(path: &Path) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_private(path: &Path) -> io::Result<fs::File> {
    fs::File::create(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp_cache(name: &str) -> Cache {
        let dir = std::env::temp_dir().join(format!("gh-stats-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Cache {
            dir: Some(dir),
            read: true,
        }
    }

    #[test]
    fn round_trip_and_expiry() {
        let c = temp_cache("rt");
        assert_eq!(c.get("k", 100), None);
        c.put("k", "{\"a\":1}\nsecond line", 100);
        assert_eq!(
            c.get("k", 100 + 5).as_deref(),
            Some("{\"a\":1}\nsecond line")
        );
        assert_eq!(c.get("k", 100 + TTL_SECS), None);
        assert_eq!(c.get("other", 100), None);
    }

    #[test]
    fn refresh_skips_reads_but_still_writes() {
        let mut c = temp_cache("refresh");
        c.read = false;
        c.put("k", "v", 1);
        assert_eq!(c.get("k", 1), None);
        c.read = true;
        assert_eq!(c.get("k", 1).as_deref(), Some("v"));
    }

    #[test]
    fn prune_removes_old_entries_only() {
        let c = temp_cache("prune");
        c.put("fresh", "v", 1);
        c.put("old", "v", 1);
        let dir = c.dir.clone().unwrap();
        std::fs::write(dir.join("notes.txt"), "keep").unwrap();

        // pretend two days passed for everything
        let later = SystemTime::now() + Duration::from_secs(2 * KEEP_SECS);
        c.prune(later);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        assert!(dir.join("notes.txt").exists());

        c.put("again", "v", 1);
        c.prune(SystemTime::now());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
    }

    #[test]
    fn no_dir_does_nothing() {
        let c = Cache {
            dir: None,
            read: true,
        };
        c.put("k", "v", 1);
        assert_eq!(c.get("k", 1), None);
    }
}
