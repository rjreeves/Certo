//! The schema and queries the generated Rust fixtures are made from (shared by the test that checks the
//! fixtures are current and the one that runs them).

pub const SCHEMA: &str = r#"
enum Role { admin, user, guest }
table customers {
    id: serial primary key
    name: text not null
    email: varchar(100) unique
    role: Role not null default user
    balance: decimal(10,2)
    born: date
    meta: json
    token: uuid
}
table orders {
    id: serial primary key
    customer_id: int not null references customers
    total: decimal(10,2) not null
    created: timestamp not null default now()
    shipped: timestamp
    seen: timestamp_naive
    qty: smallint not null default 1
    paid: bool not null default false
}
table blobs {
    id: bigserial primary key
    data: bytes
    ratio: float
    score: real
}
"#;

pub const QUERIES: &str = r#"
insert add_customer(name: text, email: text null, role: Role, balance: decimal(10,2) null, born: date null, meta: json null, token: uuid null) {
    into customers set name = :name, email = :email, role = :role, balance = :balance, born = :born, meta = :meta, token = :token
    returning id
}
query customers_by_role(r: Role null) {
    from customers c where :r is null or c.role == :r
    select c.id, c.name, c.email, c.role, c.balance, c.born, c.meta, c.token order by c.id
}
update rename_customer(id: int, name: text) { customers c set name = :name where c.id == :id }
insert place_order(customer_id: int, total: decimal(10,2), qty: smallint, paid: bool, shipped: timestamp null, seen: timestamp_naive null) {
    into orders set customer_id = :customer_id, total = :total, qty = :qty, paid = :paid, shipped = :shipped, seen = :seen
    returning id, created
}
query orders_since(since: timestamp, unused: int null) {
    from orders o where o.created >= :since
    select o.id, o.total, o.created, o.shipped, o.seen, o.qty, o.paid order by o.id
}
insert add_blob(data: bytes null, ratio: float, score: real) {
    into blobs set data = :data, ratio = :ratio, score = :score returning id
}
query blobs() { from blobs b select b.id, b.data, b.ratio, b.score order by b.id }
delete purge_orders() { from orders o all rows }
"#;

/// The same over MySQL, which has no `returning`: the inserts report a row count and the rows are read with queries.
pub const QUERIES_MYSQL: &str = r#"
insert add_customer(name: text, email: text null, role: Role, balance: decimal(10,2) null, born: date null, meta: json null, token: uuid null) {
    into customers set name = :name, email = :email, role = :role, balance = :balance, born = :born, meta = :meta, token = :token
}
query customers_by_role(r: Role null) {
    from customers c where :r is null or c.role == :r
    select c.id, c.name, c.email, c.role, c.balance, c.born, c.meta, c.token order by c.id
}
update rename_customer(id: int, name: text) { customers c set name = :name where c.id == :id }
insert place_order(customer_id: int, total: decimal(10,2), qty: smallint, paid: bool, shipped: timestamp null, seen: timestamp_naive null) {
    into orders set customer_id = :customer_id, total = :total, qty = :qty, paid = :paid, shipped = :shipped, seen = :seen
}
query orders_since(since: timestamp, unused: int null) {
    from orders o where o.created >= :since
    select o.id, o.total, o.created, o.shipped, o.seen, o.qty, o.paid order by o.id
}
insert add_blob(data: bytes null, ratio: float, score: real) {
    into blobs set data = :data, ratio = :ratio, score = :score
}
query blobs() { from blobs b select b.id, b.data, b.ratio, b.score order by b.id }
delete purge_orders() { from orders o all rows }
"#;
