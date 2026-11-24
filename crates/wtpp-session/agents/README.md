# Agent detection rules

How wt++ tells what a coding agent in a plain tab is doing (see
`src/agents.rs`): one file per agent, in herdr's manifest format, copied
unchanged from herdr (<https://github.com/herdrdev/herdr>, commit
`e8d7d9705a7a2c89cef4dd42079379f05685af87`, `src/detect/manifests/`).

herdr is licensed under the Apache License 2.0 (`LICENSE-herdr`); these files
are used under it. To update them, copy that directory's `*.toml` again and
run `cargo test -p wtpp-session agents`.
