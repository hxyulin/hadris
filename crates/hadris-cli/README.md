# Hadris CLI

This crate is the CLI of Hadris. It provides a command-line interface for Hadris.
This crate uses the hadris crate as a dependency, which means it requires std.

## Usage

```bash
$ hadris --help
```

Hadris cli uses clap to parse command-line arguments, so it supports all the features of clap, like '--help' and '--version'.

## Subcommands

### Create

Create a new Hadris image.

```bash
$ hadris create --help
```

### Write

Write a file to a Hadris image.

```bash
$ hadris write --help
```

### Read

Read a file from a Hadris image.

```bash
$ hadris read --help
```


