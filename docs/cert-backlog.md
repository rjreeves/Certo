Still evolving:

Error handling — no Result<T, E> or try/catch yet
Pattern matching depth — if let Some(x) works but how far does it go?

Generics on user-defined types (not just stdlib)
Collections beyond List — Map, Set?
String interpolation edge cases
Open questions:

Is the SQL boundary always a string, or will Certo ever get typed queries?
Module system — how do multi-file projects compose?
Package/dependency management







Major.Minor — you control in Cargo.toml
Patch — auto-increments on every release build
Date — today's date (yymmdd)
Build — increments on every build, release or debug