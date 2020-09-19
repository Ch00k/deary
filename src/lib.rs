use chrono::Utc;
use git2;
use std::collections::HashMap;
use std::env;
use std::fmt;
use std::fs::{read_dir, remove_file, File};
use std::io;
use std::io::prelude::*;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::result;
use tempfile::NamedTempFile;
use which;

const REPO_DIR: &str = ".deary";
const TMP_DIR: &str = "/dev/shm";
const GPG_ID_FILE_NAME: &str = ".gpg_id";
const GPG_OPTS: &[&str] = &[
    "--quiet",
    "--yes",
    "--compress-algo=none",
    "--no-encrypt-to",
];

type Result<T> = result::Result<T, DearyError>;

#[derive(Debug)]
enum Change {
    Add,
    Edit,
    Delete,
}

#[derive(Debug, Eq, PartialEq)]
pub struct DearyError {
    message: String,
}

impl fmt::Display for DearyError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl DearyError {
    pub fn new(msg: &str) -> DearyError {
        DearyError {
            message: msg.to_string(),
        }
    }
}

impl From<git2::Error> for DearyError {
    fn from(e: git2::Error) -> Self {
        DearyError::new(&e.to_string())
    }
}

impl From<io::Error> for DearyError {
    fn from(e: io::Error) -> Self {
        DearyError::new(&e.to_string())
    }
}

impl From<env::VarError> for DearyError {
    fn from(e: env::VarError) -> Self {
        DearyError::new(&e.to_string())
    }
}

impl From<which::Error> for DearyError {
    fn from(e: which::Error) -> Self {
        DearyError::new(&e.to_string())
    }
}

pub struct Deary {
    repo: git2::Repository,
}

impl Deary {
    pub fn init(repo_path: &Path, gpg_id: &str, git_config: HashMap<&str, &str>) -> Result<()> {
        let repo = git2::Repository::init(repo_path)?;
        let deary = Deary { repo };
        deary.set_config(git_config)?;
        deary.create_gpg_id_file(gpg_id)
    }

    pub fn new(repo_path: &Path) -> Result<Deary> {
        let repo = git2::Repository::open(repo_path)?;
        Ok(Deary { repo })
    }

    pub fn create_entry(&self) -> Result<()> {
        let tmp_file = NamedTempFile::new_in(TMP_DIR)?;
        let dt = Utc::now();
        let file_name = dt.format("%Y%m%d-%H%M%S").to_string();
        let file_path = self.repo_dir().join(&file_name);

        open_editor(&tmp_file.path())?;
        encrypt_entry(tmp_file.path(), &file_path, &self.gpg_id()?)?;
        tmp_file.close().unwrap();
        self.commit_change(&file_name, Change::Add, false)?;
        Ok(())
    }

    pub fn read_entry(&self, name: &str) -> Result<Vec<u8>> {
        let file_path = self.repo_dir().join(name);
        decrypt_entry(&file_path)
    }

    pub fn update_entry(&self, name: &str) -> Result<()> {
        let file_path = self.repo_dir().join(name);
        let text = decrypt_entry(&file_path)?;

        let mut tmp_file = NamedTempFile::new_in(TMP_DIR)?;
        tmp_file.write_all(&text)?;

        open_editor(tmp_file.path())?;
        encrypt_entry(tmp_file.path(), &file_path, &self.gpg_id()?)?;
        tmp_file.close().unwrap();
        self.commit_change(name, Change::Edit, false)?;
        Ok(())
    }

    pub fn delete_entry(&self, name: &str) -> Result<()> {
        let file_path = self.repo_dir().join(name);
        remove_file(file_path)?;
        self.commit_change(name, Change::Delete, false)
    }

    pub fn list_entries(&self) -> Result<Vec<String>> {
        let paths = read_dir(self.repo_dir())?;
        let mut file_names = vec![];

        for path in paths {
            let file_name = path?.file_name().into_string().unwrap();
            if !file_name.starts_with(".") {
                file_names.push(file_name);
            };
        }
        Ok(file_names)
    }

    fn create_gpg_id_file(&self, gpg_id: &str) -> Result<()> {
        let mut file = File::create(self.gpg_id_path())?;
        file.write_all(gpg_id.as_bytes())?;
        self.commit_change(GPG_ID_FILE_NAME, Change::Add, true)
    }

    fn set_config(&self, config: HashMap<&str, &str>) -> Result<()> {
        let mut git_config = self.repo.config()?;
        for (k, v) in &config {
            git_config.set_str(k, v)?;
        }
        Ok(())
    }

    fn commit_change(&self, file: &str, change: Change, initial: bool) -> Result<()> {
        let file_path = Path::new(file);

        let mut index = self.repo.index()?;
        match change {
            Change::Delete => index.remove_path(file_path)?,
            _ => index.add_path(file_path)?,
        }
        index.write()?;

        let oid = index.write_tree()?;
        let tree = self.repo.find_tree(oid)?;
        let signature = self.repo.signature()?;

        let mut parent_commit: Vec<&git2::Commit> = vec![];

        let commit;
        if !initial {
            let head = self.repo.head()?;
            commit = head.peel_to_commit()?;
            parent_commit.push(&commit);
        }

        self.repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            &format!("{:?} {}", change, file),
            &tree,
            &parent_commit,
        )?;

        Ok(())
    }

    fn repo_dir(&self) -> &Path {
        self.repo.workdir().unwrap()
    }

    fn gpg_id_path(&self) -> PathBuf {
        self.repo_dir().join(GPG_ID_FILE_NAME)
    }

    fn gpg_id(&self) -> Result<String> {
        let mut file = File::open(self.gpg_id_path())?;
        let mut gpg_id = String::new();
        file.read_to_string(&mut gpg_id)?;
        Ok(gpg_id)
    }
}

pub fn find_repo_path() -> PathBuf {
    let home = env::var("HOME").unwrap();
    let mut path = PathBuf::from(home);
    path.push(REPO_DIR);
    path
}

fn find_editor() -> Result<String> {
    match env::var("EDITOR") {
        Ok(editor) => Ok(editor),
        Err(_) => match which::which("vim") {
            Ok(vim) => Ok(vim.into_os_string().into_string().unwrap()),
            Err(error) => Err(DearyError::new(&format!(
                "EDITOR is not set, and vim not found in PATH ({})",
                &error
            ))),
        },
    }
}

fn find_gpg() -> Result<String> {
    match which::which("gpg") {
        Ok(gpg) => Ok(gpg.into_os_string().into_string().unwrap()),
        Err(error) => Err(DearyError::new(&format!(
            "gpg executable not found in PATH ({})",
            error
        ))),
    }
}

fn open_editor(temp_file_path: &Path) -> Result<()> {
    let editor = match find_editor() {
        Ok(e) => e,
        Err(err) => return Err(err),
    };

    let status = Command::new(&editor).arg(temp_file_path).spawn()?.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(DearyError::new(&format!("{}", status)))
    }
}

fn decrypt_entry(path: &Path) -> Result<Vec<u8>> {
    let gpg = match find_gpg() {
        Ok(g) => g,
        Err(err) => return Err(err),
    };
    Ok(Command::new(gpg)
        .args(GPG_OPTS)
        .arg("--decrypt")
        .arg(path)
        .output()?
        .stdout)
}

fn encrypt_entry(input_path: &Path, output_path: &Path, gpg_id: &str) -> Result<()> {
    let gpg = match find_gpg() {
        Ok(g) => g,
        Err(err) => return Err(err),
    };
    let status = Command::new(gpg)
        .args(GPG_OPTS)
        .arg("--encrypt")
        .arg("--recipient")
        .arg(gpg_id.trim())
        .arg("--output")
        .arg(output_path)
        .arg(input_path)
        .spawn()?
        .wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(DearyError::new(&format!("{}", status)))
    }
}
