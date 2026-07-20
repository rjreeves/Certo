# Concurrency in Certo

Certo runs concurrent work on **real OS threads** (Win32 threads on Windows,
pthreads on POSIX). You write three primitives — `spawn`, `await`, and
`parallel {}` — and the compiler generates the thread machinery for you.

Every example in this guide has been compiled and run; the outputs shown are
real.

---

## The model in one minute

| You write | What happens |
|---|---|
| `spawn f(x)` | Evaluates `x` now, then runs `f` on a **new OS thread**. Returns a *task handle* immediately — it does **not** block. |
| `await t` | Blocks until task `t` finishes, then gives you its result. |
| `parallel { a(), b() }` | Spawns every task, so they run at the same time. |
| `await parallel { a(), b() }` | Spawns all, waits for all, and returns a **tuple** of their results. |

A task handle is just a value — you can store it in a `val`, pass it around, and
`await` it later. The work between `spawn` and `await` is where you get your
parallelism: the spawned task runs while your main thread keeps going.

> **One rule to remember:** only a direct call to a *named function* is threaded
> (`spawn work(x)`). `spawn (a + b)` or `spawn someLambda()` falls back to running
> sequentially — still correct, just not concurrent.

---

## Example 1 — a single background task

Start a slow computation on another thread, do something else while it runs, then
collect the result.

```certo
module Simple

fn slowSquare(n: Int): Int = {
  // Pretend this is expensive work.
  var acc = 0
  var i = 0
  while i < 50000000 { acc = acc + (n % 7)  i = i + 1 }
  n * n
}

pub fn main(): Unit [io] = {
  val task = spawn slowSquare(9)     // starts on a new thread, returns immediately
  println("task started, doing other work...")

  val result = await task            // block here until it's done
  println(intToText(result))
}
```

Build and run:

```
$ certo run simple.cto
task started, doing other work...
81
```

Notice the message prints **before** the result — `spawn` returned right away and
`main` kept running until it hit `await`.

---

## Example 2 — two tasks in parallel, wait for both

This is the common case: you have two independent pieces of work and you want
them to run **at the same time**, then combine their results once both finish.

There are two equivalent ways to write it.

### The concise way — `parallel { … }`

`await parallel { … }` spawns every task, waits for all of them, and hands you a
tuple you can destructure:

```certo
module Parallel

fn computeA(): Int = {
  var s = 0
  var i = 0
  while i < 100000000 { s = s + (i % 3)  i = i + 1 }
  s
}

fn computeB(): Int = {
  var s = 0
  var i = 0
  while i < 100000000 { s = s + (i % 5)  i = i + 1 }
  s
}

pub fn main(): Unit [io] = {
  // Both run concurrently; we block until both are done.
  val (a, b) = await parallel { computeA(), computeB() }
  println(intToText(a + b))
}
```

```
$ certo run parallel.cto
299999999
```

Both `computeA` and `computeB` run on their own threads, so on a multi-core
machine the total time is roughly the time of the *slower* task, not the sum of
the two. (Measured: this runs ~2× faster than doing the two calls one after the
other.)

### The explicit way — `spawn` then `await`

Sometimes you want to name each task, or interleave other work between starting
them and collecting them. Spawn both **first**, then await both:

```certo
module Explicit

fn fetchUser(): Int = 100
fn fetchOrders(): Int = 42

pub fn main(): Unit [io] = {
  val userTask  = spawn fetchUser()    // both tasks are now
  val orderTask = spawn fetchOrders()  // running concurrently

  val user   = await userTask          // wait for the first
  val orders = await orderTask         // wait for the second
  println(intToText(user + orders))
}
```

```
$ certo run explicit.cto
142
```

The key is ordering: **spawn both before awaiting either.** If you wrote
`val user = await (spawn fetchUser())` then spawned the second task afterwards,
the first would finish before the second even started — that's just sequential
code.

---

## How it works under the hood

For each `spawn` call site the compiler generates:

- a small **context struct** holding the arguments and a slot for the result,
- a **worker function** that unpacks the context, calls your function, and stores
  the result,
- a `__certo_thread_spawn(...)` call (Win32 `CreateThread` / pthread
  `pthread_create`) that launches the worker.

`await` becomes a `__certo_thread_join(...)` that waits for the thread and reads
the result back out. You never see any of this — it's all in the generated C.

---

## Tips and gotchas

- **Spawn before you await.** The parallelism lives between the `spawn` and the
  `await`. Awaiting a task immediately after spawning it is just a slow way to
  call a function.
- **Only named-function calls are threaded.** `spawn doWork(x)` runs on a thread;
  `spawn (x + 1)` or spawning a lambda runs inline (sequentially).
- **Tasks should be independent.** Two tasks that both mutate the same shared
  state are a data race. Keep spawned work self-contained and combine results
  *after* `await`.
- **Results come back typed.** `await` gives you the function's real return type,
  so `val (a, b) = await parallel { f(), g() }` types `a` and `b` exactly as
  `f` and `g` return.

For the precise list of what is and isn't supported in concurrency today, see
[LIMITATIONS.md §1](LIMITATIONS.md). The higher-level structured-concurrency
features sketched in [CERTO-SPEC.md §7](CERTO-SPEC.md) (cancellation scopes,
channels, `withTimeout`, supervised background jobs) are a design target and not
implemented yet — `spawn` / `await` / `parallel {}` are what runs today.
