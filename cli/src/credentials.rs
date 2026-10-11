use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;

#[derive(Serialize, Deserialize)]
pub struct Credential {
    pub device_id: String,
    pub account_id: String,
    pub server: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
}

pub struct Store {
    path: PathBuf,
    server: String,
    _lock: File,
}

impl Store {
    pub fn open(root: PathBuf, server: &str) -> Result<Self> {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&root)?;
        let meta = fs::symlink_metadata(&root)?;
        // A shared or symlinked directory is unsafe even if the file is 0600.
        if !meta.is_dir() || meta.mode() & 0o077 != 0 || meta.uid() != unsafe { libc::geteuid() } {
            bail!(
                "{} must be a directory owned by you with mode 0700",
                root.display()
            );
        }
        let key = format!("{:x}", Sha256::digest(server.as_bytes()));
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(root.join(format!("{key}.lock")))?;
        lock.try_lock_exclusive()
            .context("Another Roundtable command is using this login")?;
        Ok(Self {
            path: root.join(format!("{key}.json")),
            server: server.into(),
            _lock: lock,
        })
    }

    fn keyring(&self) -> Result<keyring::Entry> {
        keyring::Entry::new("roundtable.cli", &self.server)
            .context("Cannot open OS credential store")
    }

    pub fn load(&self) -> Result<Option<(Credential, String)>> {
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&self.path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let meta = file.metadata()?;
        if !meta.is_file()
            || meta.permissions().mode() & 0o077 != 0
            || meta.uid() != unsafe { libc::geteuid() }
        {
            bail!("Credential file must be owned by you with mode 0600");
        }
        let credential: Credential =
            serde_json::from_reader(file).context("Invalid credential file")?;
        if credential.server != self.server {
            bail!("Credential belongs to a different server");
        }
        let token = match &credential.access_token {
            Some(token) => token.clone(),
            None => self
                .keyring()?
                .get_password()
                .context("Unlock your OS credential store and retry")?,
        };
        Ok(Some((credential, token)))
    }

    pub fn save(&self, mut credential: Credential, token: &str, file_storage: bool) -> Result<()> {
        if self.path.exists() {
            bail!("A login already exists. Run roundtable logout first.");
        }
        let mut file = tempfile::NamedTempFile::new_in(self.path.parent().unwrap())?;
        if file_storage {
            credential.access_token = Some(token.into());
        } else {
            self.keyring()?.set_password(token).context(
                "OS credential storage is unavailable. Unlock it, or retry login with --insecure-storage to use a private file",
            )?;
        }
        let mut published = false;
        let write = (|| -> Result<()> {
            serde_json::to_writer(&mut file, &credential)?;
            file.write_all(b"\n")?;
            file.as_file().sync_all()?;
            file.persist_noclobber(&self.path)?;
            published = true;
            File::open(self.path.parent().unwrap())?.sync_all()?;
            Ok(())
        })();
        if write.is_err() {
            if published {
                let _ = fs::remove_file(&self.path);
            }
            if !file_storage {
                let _ = self.keyring()?.delete_credential();
            }
        }
        write
    }

    pub fn remove(&self, credential: &Credential) -> Result<()> {
        if credential.access_token.is_none() {
            match self.keyring()?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => (),
                Err(error) => return Err(error).context("Unable to remove OS credential"),
            }
        }
        fs::remove_file(&self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_storage_is_private_locked_and_removed() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path().join("credentials");
        let store = Store::open(root.clone(), "https://roundtable.example")?;
        assert!(Store::open(root.clone(), "https://roundtable.example").is_err());
        store.save(
            Credential {
                device_id: "device".into(),
                account_id: "account".into(),
                server: "https://roundtable.example".into(),
                access_token: None,
            },
            "secret",
            true,
        )?;
        assert_eq!(fs::metadata(&root)?.permissions().mode() & 0o777, 0o700);
        assert_eq!(
            fs::metadata(&store.path)?.permissions().mode() & 0o777,
            0o600
        );
        let (credential, token) = store.load()?.unwrap();
        assert_eq!(token, "secret");
        store.remove(&credential)?;
        assert!(store.load()?.is_none());
        Ok(())
    }

    #[test]
    fn refuses_shared_directory_and_symlinked_credential() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o755))?;
        assert!(Store::open(tmp.path().into(), "https://roundtable.example").is_err());
        let store = Store::open(tmp.path().join("private"), "https://roundtable.example")?;
        std::os::unix::fs::symlink(tmp.path().join("target"), &store.path)?;
        assert!(store.load().is_err());
        Ok(())
    }
}
