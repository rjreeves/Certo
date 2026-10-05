// The same queries for MySQL, which has no `returning`: the inserts report a row count.
// Queries the code generator is checked against: parameters of several types (one nullable,
// one enum), nullable result columns, aggregates, a subquery, window functions, a union,
// a with query, and every kind of mutation.

query orders_over(min: decimal(10,2), status: Status null) {
    from orders o join customers c on o.customer_id == c.id
    where o.total >= :min and (:status is null or o.status == :status)
    select o.id, c.name as customer, o.total, o.status, o.shipped, o.paid, o.note
    order by o.id
}

query customer_stats() {
    from customers c left join orders o on o.customer_id == c.id
    group by c.id, c.name
    select c.id, c.name, count(o.id) as orders, sum(o.total) as spent
    order by c.id
}

query find_by_token(t: uuid) {
    from customers c where c.token == :t select c.id, c.born
}

query big_spenders(min: decimal(10,2)) {
    from customers c
    where (from orders o where o.customer_id == c.id select sum(o.total)) >= :min
    select c.name order by c.name
}

query ranked_orders() {
    from orders o
    select o.id, o.total,
           row_number() over (partition by o.customer_id order by o.total desc) as rn,
           sum(o.total) over (partition by o.customer_id) as customer_total
    order by o.id
}

query all_names() {
    from customers c select c.id, c.name
    union all
    from orders o select o.id, o.note
    order by id
}

query top_spender() {
    with spend as (from orders o group by o.customer_id select o.customer_id, sum(o.total) as total)
    from customers c join spend s on s.customer_id == c.id
    select c.name, s.total order by s.total desc limit 1
}

insert add_customer(name: text, email: varchar(100) null, born: date null, token: uuid null) {
    into customers set name = :name, email = :email, born = :born, token = :token
}

insert add_orders(cid: int, a: decimal(10,2), b: decimal(10,2)) {
    into orders (customer_id, total) values (:cid, :a), (:cid, :b)
}

update set_status(id: int, s: Status) {
    orders o set status = :s where o.id == :id
}

delete remove_by_status(s: Status) {
    from orders o where o.status == :s
}
