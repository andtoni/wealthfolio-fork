//! Brand logos for transaction rows.
//!
//! A merchant's logo is looked up once through Brandfetch's brand search and kept
//! in the data directory, so a phone never talks to the lookup service and a new
//! merchant gets its logo the first time it is shown. A merchant that is not
//! recognised is remembered for a week; the app shows its initials meanwhile.

use std::{path::PathBuf, sync::Arc, sync::LazyLock, time::Duration};

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use serde::Deserialize;

use crate::main_lib::AppState;

const SEARCH_URL: &str = "https://api.brandfetch.io/v2/search/";
const ICON_HOST: &str = "cdn.brandfetch.io";
const MAX_ICON_BYTES: usize = 300 * 1024;
/// How long a merchant stays remembered as unknown before it is looked up again.
const UNKNOWN_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);

static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("HTTP client")
});

#[derive(Deserialize)]
struct LogoQuery {
    name: String,
    /// The transaction's currency: a South African rand transaction prefers a `.za` brand.
    currency: Option<String>,
}

#[derive(Deserialize)]
struct Brand {
    name: Option<String>,
    domain: Option<String>,
    icon: Option<String>,
}

/// Card processors and marketplaces that prefix the real merchant's name.
const PROCESSOR_PREFIXES: [&str; 11] = [
    "DL*", "DL ", "PAYPAL *", "PP*", "PP ", "SP ", "PAYFAST*", "YOCO *", "SQ *", "TST*", "WWW.",
];

/// The words that identify a merchant: upper case, no processor prefix, no
/// reference numbers.
fn merchant_words(name: &str) -> Vec<String> {
    let mut text = name.trim().to_uppercase();
    for prefix in PROCESSOR_PREFIXES {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = rest.trim().to_string();
            break;
        }
    }
    words(&text)
}

/// A brand's name as words; "takealot.com" and "Parket.co.za" lose the domain ending.
fn brand_words(name: &str) -> Vec<String> {
    let upper = name.trim().to_uppercase();
    let without_domain = match upper.split_once('.') {
        Some((first, rest)) if !first.contains(' ') && !rest.contains(' ') => first.to_string(),
        _ => upper,
    };
    words(&without_domain)
}

/// Words that describe a payment rather than name a shop. With one of these in
/// the description, a single matching word is not taken as a brand.
const PAYMENT_WORDS: [&str; 19] = [
    "REPAYMENT",
    "PAYMENT",
    "FEES",
    "FEE",
    "TRANSFER",
    "SALARY",
    "LOAN",
    "RENT",
    "INTEREST",
    "DEPOSIT",
    "WITHDRAWAL",
    "REFUND",
    "PREMIUM",
    "TAX",
    "LEVY",
    "CHARGE",
    "CHARGES",
    "DEBIT",
    "CREDIT",
];

/// Upper-case accented letters as their plain letter, so "CAFFÈ" meets "CAFFE".
fn unaccented(c: char) -> char {
    match c {
        'À'..='Å' => 'A',
        'Ç' => 'C',
        'È'..='Ë' => 'E',
        'Ì'..='Ï' => 'I',
        'Ñ' => 'N',
        'Ò'..='Ö' | 'Ø' => 'O',
        'Ù'..='Ü' => 'U',
        other => other,
    }
}

fn words(text: &str) -> Vec<String> {
    let text: String = text.chars().map(unaccented).collect();
    text.split(|c: char| !c.is_alphanumeric() && c != '\'')
        .map(|word| word.replace('\'', ""))
        .filter(|word| !word.is_empty() && !word.chars().any(|c| c.is_ascii_digit()))
        .collect()
}

/// Whether a brand is this merchant. Its name of two or more words opens the
/// merchant's name, or its web address is the merchant's opening words run
/// together. One word alone is too weak ("Bond repayment" is not a company
/// called Bond): it counts only when the description has no payment word, and
/// then for a `.com` brand named exactly that word or a `.za` address on a rand
/// transaction.
fn is_merchant(brand: &Brand, merchant: &[String], prefer_za: bool) -> bool {
    let (Some(name), Some(domain), Some(_)) = (&brand.name, &brand.domain, &brand.icon) else {
        return false;
    };
    let name = brand_words(name);
    if name.len() >= 2 && merchant.starts_with(&name) {
        return true;
    }
    let domain = domain.to_uppercase();
    let label = domain.split('.').next().unwrap_or_default();
    let south_african = prefer_za && domain.ends_with(".ZA");
    let one_word_allowed = names_a_shop(merchant);
    (1..=merchant.len()).any(|count| {
        merchant[..count].concat() == label
            && (count >= 2
                || (one_word_allowed
                    && (south_african
                        || (domain == format!("{label}.COM") && name == merchant[..1]))))
    })
}

fn names_a_shop(merchant: &[String]) -> bool {
    !merchant
        .iter()
        .any(|word| PAYMENT_WORDS.contains(&word.as_str()))
}

/// What to search for, most specific first: the first two words, the first
/// three with any single letters ("VIDA E CAFFE"), the first word, and for a
/// rand transaction the first word as a `.co.za` address.
fn queries(merchant: &[String], prefer_za: bool) -> Vec<String> {
    let mut queries = vec![merchant
        .iter()
        .filter(|word| word.len() > 1)
        .take(2)
        .cloned()
        .collect::<Vec<_>>()
        .join(" ")];
    queries.push(
        merchant
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(" "),
    );
    if merchant.len() > 1 && merchant[0].len() >= 4 {
        queries.push(merchant[0].clone());
    }
    if prefer_za && names_a_shop(merchant) {
        queries.push(format!("{}.co.za", merchant[0].to_lowercase()));
    }
    let mut seen = Vec::new();
    queries.retain(|query| {
        !query.is_empty() && !seen.contains(query) && {
            seen.push(query.clone());
            true
        }
    });
    queries
}

/// The first brand that is this merchant; a rand transaction takes a `.za` one first.
fn pick<'a>(brands: &'a [Brand], merchant: &[String], prefer_za: bool) -> Option<&'a Brand> {
    let matching: Vec<&Brand> = brands
        .iter()
        .filter(|brand| is_merchant(brand, merchant, prefer_za))
        .collect();
    matching
        .iter()
        .find(|brand| prefer_za && brand.domain.as_deref().is_some_and(|d| d.ends_with(".za")))
        .or(matching.first())
        .copied()
}

/// The brands the service knows for a query; `None` when it could not be asked.
async fn search(query: &str) -> Option<Vec<Brand>> {
    let url = format!("{SEARCH_URL}{}", urlencoding::encode(query));
    let response = CLIENT.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json().await.ok()
}

async fn fetch_icon(url: &str) -> Option<Vec<u8>> {
    let parsed = reqwest::Url::parse(url).ok()?;
    if parsed.scheme() != "https" || parsed.host_str() != Some(ICON_HOST) {
        return None;
    }
    let response = CLIENT.get(parsed).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let bytes = response.bytes().await.ok()?;
    (!bytes.is_empty() && bytes.len() <= MAX_ICON_BYTES).then(|| bytes.to_vec())
}

enum Lookup {
    Found(Vec<u8>),
    /// The service answered and has no brand for this merchant.
    Unknown,
    /// The service could not be reached; nothing is remembered, so the next view asks again.
    Unreachable,
}

async fn look_up(merchant: &[String], prefer_za: bool) -> Lookup {
    for query in queries(merchant, prefer_za) {
        let Some(brands) = search(&query).await else {
            return Lookup::Unreachable;
        };
        if let Some(icon) =
            pick(&brands, merchant, prefer_za).and_then(|brand| brand.icon.as_deref())
        {
            return match fetch_icon(icon).await {
                Some(bytes) => Lookup::Found(bytes),
                None => Lookup::Unreachable,
            };
        }
    }
    Lookup::Unknown
}

async fn recently_written(path: &PathBuf) -> bool {
    let Ok(metadata) = tokio::fs::metadata(path).await else {
        return false;
    };
    metadata
        .modified()
        .ok()
        .and_then(|written| written.elapsed().ok())
        .is_some_and(|age| age < UNKNOWN_FOR)
}

fn image(bytes: Vec<u8>) -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/webp"),
            (header::CACHE_CONTROL, "private, max-age=2592000"),
        ],
        bytes,
    )
        .into_response()
}

fn unknown() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CACHE_CONTROL, "private, max-age=86400")],
    )
        .into_response()
}

async fn merchant_logo(
    State(state): State<Arc<AppState>>,
    Query(query): Query<LogoQuery>,
) -> Response {
    let merchant = merchant_words(&query.name);
    if merchant.is_empty() {
        return unknown();
    }
    let prefer_za = query.currency.as_deref() == Some("ZAR");
    let key = format!(
        "{}{}",
        merchant
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join("_"),
        if prefer_za { "-za" } else { "" }
    );
    let folder = PathBuf::from(&state.data_root).join("merchant-logos");
    let found = folder.join(format!("{key}.webp"));
    let missing = folder.join(format!("{key}.none"));

    if let Ok(bytes) = tokio::fs::read(&found).await {
        return image(bytes);
    }
    if recently_written(&missing).await {
        return unknown();
    }
    if tokio::fs::create_dir_all(&folder).await.is_err() {
        return unknown();
    }
    match look_up(&merchant, prefer_za).await {
        Lookup::Found(bytes) => {
            let _ = tokio::fs::write(&found, &bytes).await;
            image(bytes)
        }
        Lookup::Unknown => {
            let _ = tokio::fs::write(&missing, b"").await;
            unknown()
        }
        Lookup::Unreachable => StatusCode::NOT_FOUND.into_response(),
    }
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/merchant-logo", get(merchant_logo))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brand(name: &str, domain: &str) -> Brand {
        Brand {
            name: Some(name.to_string()),
            domain: Some(domain.to_string()),
            icon: Some(format!("https://{ICON_HOST}/{domain}")),
        }
    }

    #[test]
    fn merchant_names_lose_processor_prefixes_and_reference_numbers() {
        assert_eq!(
            merchant_words("DL*Uber Eats CPT ZA"),
            ["UBER", "EATS", "CPT", "ZA"]
        );
        assert_eq!(
            merchant_words("PAYPAL *EXAMPLESHOP 4020000000"),
            ["EXAMPLESHOP"]
        );
        assert_eq!(merchant_words("McDonald's 0000001"), ["MCDONALDS"]);
        assert_eq!(merchant_words("ELECTRICITY 01234567890"), ["ELECTRICITY"]);
    }

    #[test]
    fn a_brand_is_accepted_only_when_its_whole_name_opens_the_merchant_name() {
        let merchant = merchant_words("ATLANTIC BEVERAGE REDBU TA");
        let brands = [
            brand("Virgin Atlantic", "virginatlantic.com"),
            brand("Atlantic Technologies", "atlantic-technologies.com"),
        ];
        assert!(pick(&brands, &merchant, false).is_none());

        let merchant = merchant_words("DL*Uber Eats CPT ZA");
        let brands = [
            brand("Uber Eats", "ubereats.com"),
            brand("Uber", "uber.com"),
        ];
        assert_eq!(
            pick(&brands, &merchant, false).and_then(|b| b.domain.as_deref()),
            Some("ubereats.com")
        );

        let merchant = merchant_words("TAKEALOT SAMPLEVILLE ZA");
        let brands = [brand("takealot.com", "takealot.com")];
        assert!(pick(&brands, &merchant, false).is_some());
    }

    #[test]
    fn a_rand_transaction_prefers_the_south_african_brand() {
        let merchant = merchant_words("SAMPLEMART ONLINE SAMPLEVILLE");
        let brands = [
            brand("Samplemart", "samplemart.com.au"),
            brand("Samplemart SA", "samplemart.co.za"),
        ];
        assert_eq!(
            pick(&brands, &merchant, true).and_then(|b| b.domain.as_deref()),
            Some("samplemart.co.za")
        );
        assert!(pick(&brands, &merchant, false).is_none());
    }

    #[test]
    fn one_ordinary_word_is_not_enough_to_name_a_brand() {
        for (description, name, domain) in [
            ("Bond repayment", "Bond", "bond.org.uk"),
            ("City Power", "City", "city.fi"),
            ("School fees", "School", "school.hk"),
            ("Seed fixture marker", "Seed Health", "seed.com"),
        ] {
            let merchant = merchant_words(description);
            assert!(
                pick(&[brand(name, domain)], &merchant, true).is_none(),
                "{description}"
            );
        }

        let merchant = merchant_words("Discovery Health");
        assert!(pick(&[brand("Discovery", "discovery.co.za")], &merchant, true).is_some());
        let merchant = merchant_words("Pick n Pay");
        assert!(pick(
            &[brand("Pick n Pay Stores", "picknpay.co.za")],
            &merchant,
            true
        )
        .is_some());
    }

    #[test]
    fn a_payment_word_blocks_a_one_word_match_even_at_a_local_address() {
        let merchant = merchant_words("Bond repayment");
        assert!(pick(&[brand("Bond", "bond.co.za")], &merchant, true).is_none());
        assert!(!queries(&merchant, true).contains(&"bond.co.za".to_string()));

        let merchant = merchant_words("CHECKERS SIXTY60 SAMPLEVILLE");
        assert!(queries(&merchant, true).contains(&"checkers.co.za".to_string()));
        assert!(pick(&[brand("Checkers", "checkers.co.za")], &merchant, true).is_some());
    }

    #[test]
    fn accents_and_single_letters_do_not_hide_a_brand() {
        let merchant = merchant_words("Vida e Caffe");
        assert!(queries(&merchant, true).contains(&"VIDA E CAFFE".to_string()));
        assert!(pick(&[brand("Vida E Caffè", "vidaecaffe.com")], &merchant, true).is_some());
    }

    #[test]
    fn a_wrong_brand_with_the_same_first_word_is_not_used() {
        let merchant = merchant_words("CHECKERS SIXTY60 SAMPLEVILLE");
        let brands = [
            brand("Checkers & Rally's", "checkers.com"),
            brand("Charlotte Checkers", "charlottecheckers.com"),
        ];
        assert!(pick(&brands, &merchant, true).is_none());
    }
}
