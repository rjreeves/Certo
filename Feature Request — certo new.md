# Feature Request — certo new

## Summary

Add a `certo new <project-name>` command to the CLI
that scaffolds a new Certo project with the correct
folder structure and starter files.

## Current Situation

Developers must create folders and files manually:

    mkdir my-project
    cd my-project
    mkdir src
    mkdir db
    mkdir db\migrations
    mkdir tests
    New-Item certo.toml
    New-Item src\main.cto
    ... etc

## Requested Behaviour

    certo new my-project

Should produce:

    my-project\
    ├── certo.toml          project manifest with project name filled in
    ├── .env.example        environment variable template
    ├── .gitignore          ignores dist\ .env .certo\
    ├── README.md           basic readme with project name
    ├── src\
    │   └── main.cto        hello world entry point
    ├── db\
    │   └── migrations\     empty — ready for first migration
    ├── tests\
    │   ├── unit\
    │   └── integration\
    └── dist\               empty — build output goes here

## Generated certo.toml

    [project]
    name    = "<project-name>"
    version = "0.1.0"
    edition = "2026"

    [build]
    target = "native"
    output = "dist\\"
    entry  = "src\\main.cto"

## Generated src\main.cto

    module Main

    async fn main(): Unit =
        println("Hello from <project-name>!")

## Generated .gitignore

    dist\
    .env
    .certo\
    *.log

## Templates

Support named templates for common project types:

    certo new my-project --template api
    certo new my-project --template lib
    certo new my-project --template cli

## Priority

High — this is the first command new developers run.
First impressions matter for language adoption.

## Reference

Similar commands in other languages:
    cargo new my-project        (Rust)
    go mod init my-project      (Go)
    dotnet new console          (.NET)
    npm init my-project         (Node)