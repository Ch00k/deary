use chrono::prelude::*;
use clap::{App, Arg};
use git2::Repository;
use std::collections::HashMap;
use std::env;
use std::fs::{read_dir, remove_file, File};
use std::io::prelude::*;
use std::io::{self, Write};
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

#[derive(Debug)]
enum Change {
    Add,
    Edit,
    Delete,
}

struct Deary {
    repo: Repository,
}

impl Deary {
    fn init(repo_path: &Path, gpg_id: &str, git_config: HashMap<&str, &str>) {
        Repository::init(repo_path).unwrap();
        let deary = Deary::open(repo_path);
        deary.set_config(git_config);

        let mut file = File::create(deary.gpg_id_path()).unwrap();
        file.write_all(gpg_id.as_bytes()).unwrap();
        deary.commit_change(GPG_ID_FILE_NAME, Change::Add, true);
    }

    fn open(repo_path: &Path) -> Deary {
        let repo = Repository::open(repo_path).unwrap();
        Deary { repo }
    }

    fn set_config(&self, config: HashMap<&str, &str>) {
        let mut git_config = self.repo.config().unwrap();
        for (k, v) in &config {
            git_config.set_str(k, v).unwrap();
        }
    }

    fn commit_change(&self, file: &str, change: Change, initial: bool) {
        let file_path = Path::new(file);

        let mut index = self.repo.index().unwrap();
        match change {
            Change::Delete => index.remove_path(file_path).unwrap(),
            _ => index.add_path(file_path).unwrap(),
        }
        index.write().unwrap();

        let oid = self.repo.index().unwrap().write_tree().unwrap();
        let tree = self.repo.find_tree(oid).unwrap();
        let signature = self.repo.signature().unwrap();

        // TODO: Simplify this
        if initial {
            self.repo
                .commit(
                    Some("HEAD"),
                    &signature,
                    &signature,
                    &format!("{:?} {}", change, file),
                    &tree,
                    &[],
                )
                .unwrap();
        } else {
            let parent_commit = self.repo.head().unwrap().peel_to_commit().unwrap();
            self.repo
                .commit(
                    Some("HEAD"),
                    &signature,
                    &signature,
                    &format!("{:?} {}", change, file),
                    &tree,
                    &[&parent_commit],
                )
                .unwrap();
        }
    }

    fn gpg_id_path(&self) -> PathBuf {
        self.repo.workdir().unwrap().join(GPG_ID_FILE_NAME)
    }

    fn gpg_id(&self) -> String {
        let mut file = File::open(self.gpg_id_path()).unwrap();
        let mut gpg_id = String::new();
        file.read_to_string(&mut gpg_id).unwrap();
        gpg_id
    }

    fn create_entry(&self) {
        let tmp_file = NamedTempFile::new_in(TMP_DIR).unwrap();
        let dt = Utc::now();
        let file_name = dt.format("%Y%m%d-%H%M%S").to_string();
        let file_path = self.repo.workdir().unwrap().join(&file_name);

        open_editor(&tmp_file.path());
        encrypt_entry(tmp_file.path(), &file_path, &self.gpg_id());
        tmp_file.close().unwrap();
        self.commit_change(&file_name, Change::Add, false);
    }

    fn read_entry(&self, name: &str) -> Vec<u8> {
        let file_path = self.repo.workdir().unwrap().join(name);
        decrypt_entry(&file_path)
    }

    fn update_entry(&self, name: &str) {
        let file_path = self.repo.workdir().unwrap().join(name);
        let text = decrypt_entry(&file_path);

        let mut tmp_file = NamedTempFile::new_in(TMP_DIR).unwrap();
        tmp_file.write_all(&text).unwrap();

        open_editor(tmp_file.path());
        encrypt_entry(tmp_file.path(), &file_path, &self.gpg_id());
        tmp_file.close().unwrap();
        self.commit_change(name, Change::Edit, false);
    }

    fn delete_entry(&self, name: &str) {
        let file_path = self.repo.workdir().unwrap().join(name);
        remove_file(file_path).unwrap();
        self.commit_change(name, Change::Delete, false);
    }

    fn list_entries(&self) {
        let paths = read_dir(self.repo.workdir().unwrap()).unwrap();
        for path in paths {
            let file = path.unwrap().file_name();
            if !file.to_str().unwrap().starts_with(".") {
                println!("{}", file.to_str().unwrap());
            }
        }
    }
}

fn open_editor(temp_file_path: &Path) {
    Command::new("vim")
        .arg(temp_file_path)
        .spawn()
        .expect("failed to run vim")
        .wait()
        .expect("vim returned an error");
}

fn decrypt_entry(path: &Path) -> Vec<u8> {
    Command::new("gpg")
        .args(GPG_OPTS)
        .arg("--decrypt")
        .arg(path)
        .output()
        .expect("failed to run gpg")
        .stdout
}

fn encrypt_entry(input_path: &Path, output_path: &Path, gpg_id: &str) {
    Command::new("gpg")
        .args(GPG_OPTS)
        .arg("--encrypt")
        .arg("--recipient")
        .arg(gpg_id.trim())
        .arg("--output")
        .arg(output_path)
        .arg(input_path)
        .spawn()
        .expect("failed to run gpg")
        .wait()
        .expect("gpg returned an error");
}

fn find_repo_path() -> PathBuf {
    let home = env::var("HOME").unwrap();
    let mut path = PathBuf::from(home);
    path.push(".deary");
    path
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
                eprintln!("Repository {} already exists", repo_path.display());
                std::process::exit(1);
            }
            let mut git_config = HashMap::new();
            git_config.insert("user.name", "noname");
            git_config.insert("user.email", "noemail");
            Deary::init(&repo_path, init.value_of("key_id").unwrap(), git_config);
        }
        ("create", Some(_)) => {
            Deary::open(&find_repo_path()).create_entry();
        }
        ("show", Some(show)) => {
            let text = Deary::open(&find_repo_path()).read_entry(show.value_of("name").unwrap());
            io::stdout().write_all(&text).unwrap();
        }
        ("edit", Some(edit)) => {
            Deary::open(&find_repo_path()).update_entry(edit.value_of("name").unwrap());
        }
        ("delete", Some(delete)) => {
            Deary::open(&find_repo_path()).delete_entry(delete.value_of("name").unwrap());
        }
        ("list", Some(_)) => {
            Deary::open(&find_repo_path()).list_entries();
        }
        _ => {}
    }
}
