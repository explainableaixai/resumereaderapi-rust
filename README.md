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

## FAQ

**Why a `ParseOptions` struct?** Seven optional fields are easier to read than seven positional arguments.

**Can I use blocking code?** Wrap calls in a Tokio runtime and `block_on`.

**License?** MIT. Contact info@alpha-quantum.com.
