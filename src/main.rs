use chrono::prelude::*;
use clap::{App, Arg};
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
use tempfile::NamedTempFile;

const TMP_DIR: &str = "/dev/shm";
const GPG_ID_FILE_NAME: &str = ".gpg_id";
const GPG_OPTS: &[&str] = &[
    "--quiet",
    "--yes",
    "--compress-algo=none",
    "--no-encrypt-to",
];

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

#[derive(Debug)]
enum Change {
    Add,
    Edit,
    Delete,
}

struct Deary {
    repo: git2::Repository,
}

impl Deary {
    fn init(
        repo_path: &Path,
        gpg_id: &str,
        git_config: HashMap<&str, &str>,
    ) -> Result<(), DearyError> {
        let repo = git2::Repository::init(repo_path)?;
        let deary = Deary::open_from_repo(repo);
        deary.set_config(git_config)?;

        let mut file = File::create(deary.gpg_id_path())?;
        file.write_all(gpg_id.as_bytes())?;
        deary.commit_change(GPG_ID_FILE_NAME, Change::Add, true)
    }

    fn open_from_path(repo_path: &Path) -> Result<Deary, DearyError> {
        let repo = git2::Repository::open(repo_path)?;
        Ok(Deary { repo })
    }

    fn open_from_repo(repo: git2::Repository) -> Deary {
        Deary { repo }
    }

    fn set_config(&self, config: HashMap<&str, &str>) -> Result<(), DearyError> {
        let mut git_config = self.repo.config()?;
        for (k, v) in &config {
            git_config.set_str(k, v)?;
        }
        Ok(())
    }

    fn commit_change(&self, file: &str, change: Change, initial: bool) -> Result<(), DearyError> {
        let file_path = Path::new(file);

        let mut index = self.repo.index()?;
        match change {
            Change::Delete => index.remove_path(file_path)?,
            _ => index.add_path(file_path)?,
        }
        index.write()?;

        let oid = self.repo.index().unwrap().write_tree()?;
        let tree = self.repo.find_tree(oid)?;
        let signature = self.repo.signature()?;

        // TODO: Simplify this
        if initial {
            self.repo.commit(
                Some("HEAD"),
                &signature,
                &signature,
                &format!("{:?} {}", change, file),
                &tree,
                &[],
            )?;
        } else {
            let parent_commit = self.repo.head().unwrap().peel_to_commit().unwrap();
            self.repo.commit(
                Some("HEAD"),
                &signature,
                &signature,
                &format!("{:?} {}", change, file),
                &tree,
                &[&parent_commit],
            )?;
        }
        Ok(())
    }

    fn repo_dir(&self) -> &Path {
        self.repo.workdir().unwrap()
    }

    fn gpg_id_path(&self) -> PathBuf {
        self.repo_dir().join(GPG_ID_FILE_NAME)
    }

    fn gpg_id(&self) -> Result<String, DearyError> {
        let mut file = File::open(self.gpg_id_path())?;
        let mut gpg_id = String::new();
        file.read_to_string(&mut gpg_id)?;
        Ok(gpg_id)
    }

    fn create_entry(&self) -> Result<(), DearyError> {
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

    fn read_entry(&self, name: &str) -> Result<Vec<u8>, DearyError> {
        let file_path = self.repo_dir().join(name);
        decrypt_entry(&file_path)
    }

    fn update_entry(&self, name: &str) -> Result<(), DearyError> {
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

    fn delete_entry(&self, name: &str) -> Result<(), DearyError> {
        let file_path = self.repo_dir().join(name);
        remove_file(file_path)?;
        self.commit_change(name, Change::Delete, false)
    }

    fn list_entries(&self) -> Result<Vec<String>, DearyError> {
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
}

fn open_editor(temp_file_path: &Path) -> Result<(), DearyError> {
    let status = Command::new("vim").arg(temp_file_path).spawn()?.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(DearyError::new(&format!("{}", status)))
    }
}

fn decrypt_entry(path: &Path) -> Result<Vec<u8>, DearyError> {
    Ok(Command::new("gpg")
        .args(GPG_OPTS)
        .arg("--decrypt")
        .arg(path)
        .output()?
        .stdout)
}

fn encrypt_entry(input_path: &Path, output_path: &Path, gpg_id: &str) -> Result<(), DearyError> {
    let status = Command::new("gpg")
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

fn find_repo_path() -> PathBuf {
    let home = env::var("HOME").unwrap();
    let mut path = PathBuf::from(home);
    path.push(".deary");
    path
}

fn exit_with_error(error: DearyError) {
    eprintln!("{}", error);
    std::process::exit(1);
}

fn main() {
    let deary = App::new("deary")
        .version("0.1.0")
        .author("AY")
        .about("dear diary")
        .subcommand(
            App::new("init")
                .about("init")
                .arg(Arg::with_name("key_id").about("GPG key ID").required(true)),
        )
        .subcommand(App::new("list").about("list"))
        .subcommand(
            App::new("show")
                .about("show")
                .arg(Arg::with_name("name").about("Entry name").required(true)),
        )
        .subcommand(App::new("create").about("create"))
        .subcommand(
            App::new("edit")
                .about("edit")
                .arg(Arg::with_name("name").about("Entry name").required(true)),
        )
        .subcommand(
            App::new("delete")
                .about("delete")
                .arg(Arg::with_name("name").about("Entry name").required(true)),
        )
        .get_matches();

    match deary.subcommand() {
        ("init", Some(init)) => {
            let repo_path = find_repo_path();
            if repo_path.exists() {
                exit_with_error(DearyError::new(&format!(
                    "Repository {} already exists",
                    repo_path.display()
                )));
                std::process::exit(1);
            }

            let mut git_config = HashMap::new();
            git_config.insert("user.name", "noname");
            git_config.insert("user.email", "noemail");
            if let Err(e) = Deary::init(&repo_path, init.value_of("key_id").unwrap(), git_config) {
                exit_with_error(e);
            }
        }
        ("create", Some(_)) => {
            match Deary::open_from_path(&find_repo_path()) {
                Ok(deary) => {
                    if let Err(e) = deary.create_entry() {
                        exit_with_error(e);
                    }
                }
                Err(e) => exit_with_error(e),
            };
        }
        ("show", Some(show)) => {
            match Deary::open_from_path(&find_repo_path()) {
                Ok(deary) => match deary.read_entry(show.value_of("name").unwrap()) {
                    Ok(text) => {
                        if let Err(e) = io::stdout().write_all(&text) {
                            exit_with_error(DearyError::from(e))
                        }
                    }
                    Err(e) => exit_with_error(e),
                },

                Err(e) => exit_with_error(e),
            };
        }
        ("edit", Some(edit)) => {
            match Deary::open_from_path(&find_repo_path()) {
                Ok(deary) => {
                    if let Err(e) = deary.update_entry(edit.value_of("name").unwrap()) {
                        exit_with_error(e);
                    }
                }
                Err(e) => exit_with_error(e),
            };
        }
        ("delete", Some(delete)) => {
            match Deary::open_from_path(&find_repo_path()) {
                Ok(deary) => {
                    if let Err(e) = deary.delete_entry(delete.value_of("name").unwrap()) {
                        exit_with_error(e);
                    }
                }
                Err(e) => exit_with_error(e),
            };
        }
        ("list", Some(_)) => {
            match Deary::open_from_path(&find_repo_path()) {
                Ok(deary) => match deary.list_entries() {
                    Ok(entries) => {
                        for e in entries {
                            println!("{}", e);
                        }
                    }
                    Err(e) => exit_with_error(e),
                },
                Err(e) => exit_with_error(e),
            };
        }
        _ => {}
    }
}
