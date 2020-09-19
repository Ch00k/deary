mod lib;

use clap::{crate_authors, crate_description, crate_version, App, Arg};
use lib::{find_repo_path, Deary, DearyError};
use std::collections::HashMap;
use std::env;
use std::io;
use std::io::prelude::*;

fn exit_with_error(error: DearyError) {
    eprintln!("{}", error);
    std::process::exit(1);
}

fn main() {
    let deary = App::new("deary")
        .version(crate_version!())
        .author(crate_authors!())
        .about(crate_description!())
        .subcommand(
            App::new("init").about("Initialize a new diary").arg(
                Arg::with_name("key_id")
                    .about("GPG key ID (or email address, associated with the key)")
                    .required(true),
            ),
        )
        .subcommand(App::new("list").about("List diary entries"))
        .subcommand(
            App::new("show")
                .about("Show a diary entry")
                .arg(Arg::with_name("name").about("Entry name").required(true)),
        )
        .subcommand(App::new("create").about("Create a new diary entry"))
        .subcommand(
            App::new("edit")
                .about("Edit a diary entry")
                .arg(Arg::with_name("name").about("Entry name").required(true)),
        )
        .subcommand(
            App::new("delete")
                .about("Delete a diary entry")
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
            match Deary::new(&find_repo_path()) {
                Ok(deary) => {
                    if let Err(e) = deary.create_entry() {
                        exit_with_error(e);
                    }
                }
                Err(e) => exit_with_error(e),
            };
        }
        ("show", Some(show)) => {
            match Deary::new(&find_repo_path()) {
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
            match Deary::new(&find_repo_path()) {
                Ok(deary) => {
                    if let Err(e) = deary.update_entry(edit.value_of("name").unwrap()) {
                        exit_with_error(e);
                    }
                }
                Err(e) => exit_with_error(e),
            };
        }
        ("delete", Some(delete)) => {
            match Deary::new(&find_repo_path()) {
                Ok(deary) => {
                    if let Err(e) = deary.delete_entry(delete.value_of("name").unwrap()) {
                        exit_with_error(e);
                    }
                }
                Err(e) => exit_with_error(e),
            };
        }
        ("list", Some(_)) => {
            match Deary::new(&find_repo_path()) {
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
