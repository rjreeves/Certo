Still evolving:

Generics on user-defined types (not just stdlib)
Collections beyond List — Map, Set?
String interpolation edge cases
Open questions:

Is the SQL boundary always a string, or will Certo ever get typed queries?
Module system — how do multi-file projects compose?
Package/dependency management


Extending certo db pull to report views, functions, sequences  --- Maybe.
SQL string validation (column/table names in query strings)
Typed queries


? propagation operator (defer):

fn readAndParse(path: Text): Result<Int, Text> [io] = {
    val content = readFile(path)?   // returns Err early if Err
    val n = parseInt(content)?
    Ok(n)
}






