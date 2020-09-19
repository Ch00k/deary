# deary

**`deary`** is a personal secure diary. It was inspired by
[`pass`](https://www.passwordstore.org), the standard unix password manager. In deary the
entire diary is a Git repository, and each entry is encrypted with GPG.

Due to the fact that `deary` uses `/dev/shm` for temporary files, only Linux is supported
at the moment.

## Installation

Download the latest release [here](https://github.com/Ch00k/deary/releases), and add the
downloaded file to your `$PATH`, optionally renaming it to `deary`.

## Usage

Initialize a new repository:

```
$ deary init <your_GPG_key_ID>
```

This will create a new repository at `~/.deary`.

Create your first entry:

```
$ deary create
```

This will open `vim`, allowing you yo type in your entry. When done, simply close `vim`
saving the changes, and the entry will be encrypted and committed to the repository. The
filename of the entry will be set to the current UTC timestamp.

For more information on usage run

```
$ deary help
```
