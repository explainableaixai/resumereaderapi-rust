# resumereaderapi for Rust

An async client that sends resumes to the Resume Reader API and returns the parsed candidate as JSON. Normalization of job titles, skills and locations is included.

```toml
[dependencies]
resumereaderapi = "0.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

TLS uses rustls. Responses are `serde_json::Value`.

## Hello, candidate

```rust
use resumereaderapi::{Client, ParseOptions};

#[tokio::main]
async fn main() -> Result<(), resumereaderapi::Error> {
    let client = Client::new(std::env::var("RESUME_KEY").unwrap());
    let out = client.parse_file("cv.pdf", &ParseOptions::default()).await?;

    let r = &out["resume"];
    println!("{} | {}", r["contact"]["full_name"], r["career"]["seniority_level"]);
    Ok(())
}
```

## API at a glance

| Function | Notes |
|---|---|
| `parse_text(text, &opts)` | UTF-8 text |
| `parse_file(path, &opts)` | reads the file and base64 encodes it |
| `parse_url(url, &opts)` | the service fetches an https link |
| `normalize_titles(&[String])` | batched in groups of 100 |
| `normalize_skills(&[String])` | same |
| `normalize_locations(&[String])` | same |

`ParseOptions` has `field_names`, `exclude_sensitive`, `anonymize`, `max_pages`, `sections` and `language`. It implements `Default`, so `..Default::default()` keeps call sites short.

## Matching errors

```rust
use resumereaderapi::Error;

match client.parse_text(&cv, &Default::default()).await {
    Ok(v) => println!("{}", v["resume"]["contact"]["full_name"]),
    Err(Error::Api { status: 402, .. }) => eprintln!("credits exhausted"),
    Err(Error::Api { status: 429, .. }) => eprintln!("slow down"),
    Err(Error::Api { status, message, .. }) => eprintln!("{status}: {message}"),
    Err(e) => eprintln!("{e}"),
}
```

HTTP is always 200 from this service. The crate converts the body status into `Error::Api`, so you never need to inspect the transport code.

## Feeding an applicant tracking system

A common pipeline is: webhook receives a file, the file is parsed, fields are mapped to ATS columns. The mapping is the interesting part.

| ATS column | Resume field |
|---|---|
| name | `contact.full_name` |
| email | `contact.emails[0]` |
| location | `contact.address.city` |
| title | `career.current_position.title` |
| tenure | `career.average_tenure_months` |
| skills | `skills.technical` |

This is the core of [ATS enrichment](https://www.resumereaderapi.com/use-cases/ats-enrichment.php) for teams that already hold thousands of untouched CVs.

## Normalize messy locations

```rust
let places = vec!["NYC".to_string(), "München".to_string(), "Remote (Germany)".to_string()];
let out = client.normalize_locations(&places).await?;
for r in &out.results {
    println!("{} -> {} (remote: {})", r["input"], r["normalized"], r["remote"]);
}
println!("spent {} credits", out.credits_used);
```

Ambiguous input such as a bare town name keeps `country` empty rather than guessing. The rules are described under [location normalization](https://www.resumereaderapi.com/normalization/locations.php).

## Test results you can read

The vendor publishes a checklist of tested behaviours at [quality testing](https://www.resumereaderapi.com/quality-testing.php), including every error path.

## Practical notes

- 30 requests per 60 seconds per IP. Use a semaphore when you loop.
- 10 MB per file; 20 pages per document.
- Keep the key on the server.

<!--expanded-->
## Why Rust for resume pipelines

Resume processing looks like a text problem and turns out to be a plumbing problem. Files arrive from email, uploads and archives. They have to be queued, sent, retried, stored and indexed without losing any of them. Rust is a good tool for plumbing: predictable memory, strong error handling and easy concurrency. The crate keeps its own part small so your pipeline can stay in charge.

The client is a thin layer over one HTTPS endpoint with JSON bodies. It reads the status field from the response body, because the HTTP status is always 200 for this service, and converts non-200 outcomes into `Error::Api`. Everything else is up to you.

## A queue based ingestion service

A realistic ingestion service has four stages: receive, enqueue, parse, store. Here is how the parse stage looks with a bounded channel and a fixed number of workers:

```rust
use resumereaderapi::{Client, ParseOptions};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::mpsc;

async fn run(client: Arc<Client>, mut rx: mpsc::Receiver<PathBuf>) {
    let (done_tx, mut done_rx) = mpsc::channel::<(String, serde_json::Value)>(64);

    let producer = tokio::spawn(async move {
        while let Some(path) = rx.recv().await {
            match client.parse_file(&path, &ParseOptions::default()).await {
                Ok(v) => { let _ = done_tx.send((path.display().to_string(), v)).await; }
                Err(e) => eprintln!("{}: {e}", path.display()),
            }
            tokio::time::sleep(Duration::from_millis(2100)).await;
        }
    });

    while let Some((name, value)) = done_rx.recv().await {
        println!("{name}: {}", value["resume"]["contact"]["full_name"]);
    }
    let _ = producer.await;
}
```

The two second pause per file keeps a single IP under 30 requests per 60 seconds. If you run from several machines, give each its own pacing, or share a limiter through your queue.

## Mapping the response into your own types

You will want typed access to the fields you store. Define structs and deserialize the part of the response you need:

```rust
#[derive(serde::Deserialize, Debug)]
struct Candidate {
    contact: Contact,
    career: Career,
    skills: Skills,
}

#[derive(serde::Deserialize, Debug)]
struct Contact { full_name: Option<String>, emails: Vec<String> }

#[derive(serde::Deserialize, Debug)]
struct Career {
    seniority_level: Option<String>,
    total_experience_years: Option<f64>,
    average_tenure_months: Option<u32>,
}

#[derive(serde::Deserialize, Debug)]
struct Skills { technical: Vec<String> }

let out = client.parse_text(&text, &Default::default()).await?;
let candidate: Candidate = serde_json::from_value(out["resume"].clone())?;
```

Every key exists in every response, so these structs can use plain `Vec` for arrays and `Option` for scalars. That simplicity is a direct result of the schema rule that missing evidence becomes `null` or an empty array.

## Normalization at scale

Normalizers shine on large columns. Pull the distinct values from your database, send them in batches, and write the canonical forms back. Because each call handles up to a hundred items and the client splits larger lists for you, a single function call can clean a column:

```rust
let titles: Vec<String> = distinct_titles_from_db().await;
let out = client.normalize_titles(&titles).await?;
for row in out.results {
    save_title_mapping(row["input"].as_str().unwrap(), &row["normalized_title"], &row["seniority_level"]).await;
}
println!("{} credits", out.credits_used);
```

Check the `method` field in each row. It tells you whether a result came from the deterministic rules or the fallback, and `confidence` gives a number between zero and one. A reasonable policy is to accept everything and review rows below a threshold of your choosing.

## Location data for search

A radius search needs coordinates, and this service does not return coordinates. It returns structured places: city, region, country, ISO code, a metro area and a remote flag. That is enough to filter by country, to group by metro and to separate remote candidates from local ones. If you need distances, join the canonical city and country to a gazetteer of your own.

The page on [contextual targeting explained](https://www.cookielessaudiences.com/features/contextual-vs-behavioral-targeting.php) comes from a sibling service and is a useful read if you also advertise open roles and want to understand how placements can be chosen without tracking people.

Teams that evaluate recruiting firms for acquisition can read about [add-on sourcing for PE platforms](https://www.acquisitionuniverse.com/for/private-equity-platforms.php), which covers how a buy and build programme finds independent companies in a fragmented vertical.

## Error taxonomy

- 400: the request has no input, a bad file or an oversized batch. Fix the call.
- 401: the key is wrong.
- 402: not enough credits. Nothing is billed. Alert someone.
- 413: more than 20 pages, about 30,000 tokens or 10 MB. Split or trim.
- 422: no readable text. Ask for another file.
- 429: slow down.

Map them to retry or no retry in one place, so the rule is easy to find and change.

## Release notes

Version 0.1.0 is the first release. The crate has a small public surface and a test for the payload builder. Expect additions in the 0.x series, and pin your version in `Cargo.toml`.

<!--extra-->
## Deployment notes

Build with the release profile and a static target if your containers are minimal. The crate uses rustls, so no system TLS library is needed. Set the key through the environment, set timeouts on every task and run at least one live smoke test after every deployment. Keep the credit balance in a metric and alert on it. These four habits keep a small ingestion service dependable for a long time.

Also set a policy for what happens to files that fail. A folder of failures reviewed every week is far more valuable than a retry loop that never ends.

<!--further-->
## Further reading and practical notes

New Rust users will find the [Rust language site](https://www.rust-lang.org/) a useful guide to tooling and the ecosystem. The standard of occupations and skills that the `esco` block aligns to is explained on the [ESCO portal](https://esco.ec.europa.eu/en), which also lets you look up codes by name.

Keep two habits in a Rust service that parses resumes. First, make the credit balance visible. The response carries `remaining_credits`, so write it to a metric on every call, and alert when it falls below a level you choose. A pipeline that runs dry at three in the morning wastes the night shift of everyone who depends on it. Second, keep a dead letter folder. When a file fails with 413 or 422, move it there with a text file recording the status, and review the folder weekly. You will learn which senders produce unreadable scans, and that knowledge is worth more than any retry policy.

Finally, be careful with logs. Resumes contain names, emails and phone numbers. Log the file name and the status, never the content, and make sure error messages from your own code do not include the parsed fields.

## FAQ

**Why a `ParseOptions` struct?** Seven optional fields are easier to read than seven positional arguments.

**Can I use blocking code?** Wrap calls in a Tokio runtime and `block_on`.

**License?** MIT. Contact info@alpha-quantum.com.
