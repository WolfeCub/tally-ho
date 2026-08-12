//! Who owes what, guessed from how people describe themselves.
//!
//! If Josh is the only one who drinks beer, the beer on a receipt is his. If Ash
//! is vegetarian the steak isn't hers, and with only the two of them it's Josh's.
//! That's reading English rather than arithmetic, so a model does it, off the
//! descriptions typed into the settings screen.
//!
//! A guess is not an answer. It's written with its reason attached, so the review
//! screen can show it as a guess, and it never lands on an item a human has
//! already decided.

use std::time::{Duration, Instant};

use anyhow::Context as _;
use rig_agent::agent::{Agent, OutputMode};
use rig_agent::prelude::*;
use rig_core::message::Message;
use rig_core::providers::ollama;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::server::{ask, extract, models, query};
use crate::shared::dto;

/// Roughly 25 output tokens an item, so this covers a long grocery receipt. It's
/// a backstop against a model that won't stop rather than a real limit.
const MAX_OUTPUT_TOKENS: u64 = 2048;

/// Shorter than extraction's: there's no photo to read, just a list of items.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// What the model is asked for, one entry per item.
///
/// `person` is a name off the roster and nothing else. See [`resolve`], which
/// throws away anything it doesn't recognise.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Assignments {
    pub assignments: Vec<Assigned>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Assigned {
    /// The item's number, as the question listed them.
    pub item: u32,
    /// The reasoning, which has to be written before [`Self::person`] so the name
    /// comes out of it rather than being justified after the fact.
    ///
    /// Named to sort before it: fields are generated in schema order and schemars
    /// sorts them alphabetically, so as `why` it came last, and anything needing
    /// working out came back null with the working underneath it.
    pub because: Option<String>,
    pub person: Option<String>,
}

/// A name the model put to one item, ready to write.
#[derive(Debug, Clone, PartialEq)]
pub struct Guess {
    pub person_id: Uuid,
    pub why: String,
}

/// Ollama drops schema `description` fields when it compiles the grammar, so
/// what the fields mean has to be said here. Same reason as in
/// [`crate::server::extract`].
const PROMPT: &str = "\
You work out which of the people sharing a card each item on a receipt is for.

You are given the people, how each of them is described, and the numbered items. \
Reply with one entry per item, filling the fields in this order:
- item: the item's number.
- because: go through every person by name, say whether the descriptions rule \
them out for that item, and finish with who that leaves. Always fill this in \
before you name anybody.
- person: who the item is for, spelled exactly as given, or null.

Go through the items one at a time. There are two ways an item is somebody's, and \
either one is enough.

Their own description fits the item. That settles it on its own, whether or not \
anybody else is ruled out.

Or every other person is ruled out for it, which settles it just as well, and you \
don't also need something saying they're the one who buys that sort of thing. \
Somebody is ruled out when the item is at odds with their description, and what \
somebody does, avoids or can't have covers everything of that kind rather than \
only the words it uses.

Anything else is null, which is the answer for everyday and household things.

Never invent a name, and never go on the item alone. Ruling somebody out and \
naming somebody both have to come from the descriptions, because a wrong name \
puts the money on the wrong person.";

#[async_trait::async_trait]
pub trait ItemAssigner: Send + Sync {
    /// `items` are descriptions in the order the reply's numbers refer to.
    async fn assign(
        &self,
        merchant: &str,
        people: &[dto::Person],
        items: &[&str],
    ) -> anyhow::Result<Vec<Assigned>>;
}

#[derive(Debug, Clone)]
pub struct Config {
    pub url: String,
    pub model: String,
    pub keep_alive: Option<String>,
}

impl Config {
    /// Shares Ollama with extraction, and by default its model too: the one that
    /// structures a receipt is already a general text model. Override with
    /// `OLLAMA_ASSIGN_MODEL`.
    pub fn from_extraction(extraction: &extract::Config) -> Self {
        Self {
            url: extraction.url.clone(),
            model: crate::server::env::string("OLLAMA_ASSIGN_MODEL", &extraction.model),
            keep_alive: extraction.keep_alive.clone(),
        }
    }
}

pub struct OllamaAssigner {
    agent: Agent<ollama::CompletionModel>,
    model: String,
}

impl OllamaAssigner {
    pub fn new(config: Config) -> Result<Self, ask::Error> {
        // `think` off, temperature 0 and a Native schema for the same reasons as
        // extraction, which documents each of them.
        let agent = ask::client(&config.url)?
            .agent(&config.model)
            .preamble(PROMPT)
            .temperature(0.0)
            .additional_params(ask::options(
                config.keep_alive.as_deref(),
                serde_json::json!({ "think": false }),
            ))
            .max_tokens(MAX_OUTPUT_TOKENS)
            .output_schema::<Assignments>()
            .output_mode(OutputMode::Native)
            .build();

        Ok(Self {
            agent,
            model: config.model,
        })
    }
}

#[async_trait::async_trait]
impl ItemAssigner for OllamaAssigner {
    async fn assign(
        &self,
        merchant: &str,
        people: &[dto::Person],
        items: &[&str],
    ) -> anyhow::Result<Vec<Assigned>> {
        let started = Instant::now();
        let message = Message::user(question(merchant, people, items));
        let raw = ask::once(&self.agent, message, REQUEST_TIMEOUT, None).await?;

        let said: Assignments = serde_json::from_str(&raw)
            .context("model returned data that did not match the schema")?;
        tracing::debug!(
            model = %self.model,
            elapsed_s = started.elapsed().as_secs_f32(),
            entries = said.assignments.len(),
            "guessed assignments"
        );
        Ok(said.assignments)
    }
}

/// The roster and the items, as the model sees them. Numbered from 1, which is
/// how [`resolve`] reads them back.
fn question(merchant: &str, people: &[dto::Person], items: &[&str]) -> String {
    let mut out = String::from("People:\n");
    for person in people {
        // Everyone is listed, described or not: an item can be theirs by
        // elimination.
        let about = person
            .description
            .as_deref()
            .map(str::trim)
            .filter(|described| !described.is_empty())
            .unwrap_or("nothing said about them");
        out.push_str(&format!("- {}: {about}\n", person.name));
    }

    out.push_str(&format!("\nReceipt from {}:\n", merchant.trim()));
    for (i, item) in items.iter().enumerate() {
        out.push_str(&format!("{}. {item}\n", i + 1));
    }
    out
}

/// What the model said, lined up with the items it was asked about.
///
/// One slot per item. Anything it left out, numbered at an item that isn't there,
/// or named nobody by comes back `None`, and so does a name nobody on the roster
/// answers to, which is the model making somebody up.
pub fn resolve(people: &[dto::Person], items: usize, said: &[Assigned]) -> Vec<Option<Guess>> {
    let mut out = vec![None; items];

    for entry in said {
        let Some(name) = entry
            .person
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        else {
            continue;
        };
        // Numbered from 1 in the question.
        let Some(index) = (entry.item as usize).checked_sub(1).filter(|i| *i < items) else {
            tracing::debug!(item = entry.item, "guess for an item that isn't there");
            continue;
        };
        let Some(person) = people
            .iter()
            .find(|p| p.name.trim().eq_ignore_ascii_case(name))
        else {
            tracing::debug!(%name, "guess for somebody who isn't on the roster");
            continue;
        };
        // First word wins if it says something twice.
        if out[index].is_some() {
            continue;
        }

        let why = entry
            .because
            .as_deref()
            .map(str::trim)
            .filter(|why| !why.is_empty())
            .unwrap_or("from their description");
        out[index] = Some(Guess {
            person_id: person.id,
            why: why.to_string(),
        });
    }

    out
}

/// Guesses who owes what on one receipt and writes it, returning how many items
/// it put a name to.
///
/// Asks nothing when there's nothing to be gained: nobody described, no items, or
/// every item already somebody's answer. On a fresh install that's every upload.
pub async fn suggest(
    db: &mut toasty::Db,
    assigner: &dyn ItemAssigner,
    receipt_id: Uuid,
) -> anyhow::Result<usize> {
    let people = query::list_people(db).await?;
    if !people.iter().any(dto::Person::described) {
        return Ok(0);
    }

    let receipt = models::Receipt::get_by_id(db, &receipt_id).await?;
    let mut items = receipt.line_items().exec(db).await?;
    // The receipt's own order, so the numbers mean the same thing on both sides.
    items.sort_by_key(|item| item.position);
    if items.iter().all(decided) {
        return Ok(0);
    }

    let descriptions: Vec<&str> = items.iter().map(|i| i.description.as_str()).collect();
    let said = assigner
        .assign(&receipt.merchant, &people, &descriptions)
        .await?;

    let guesses = resolve(&people, items.len(), &said);
    let named = query::apply_guesses(db, items, guesses).await?;
    tracing::info!(%receipt_id, named, "guessed who owes what");
    Ok(named)
}

/// Whether a human has had their say about this one, which the model's guess is
/// never allowed to overrule.
pub fn decided(item: &models::LineItem) -> bool {
    item.person_id.is_some() && item.guessed_why.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(n: u128, name: &str, description: Option<&str>) -> dto::Person {
        dto::Person {
            id: Uuid::from_u128(n),
            name: name.into(),
            description: description.map(str::to_string),
        }
    }

    fn pair() -> Vec<dto::Person> {
        vec![
            person(1, "Josh", Some("drinks all the beer")),
            person(2, "Ash", Some("vegetarian")),
        ]
    }

    /// What the model said about one item.
    fn said(item: u32, person: Option<&str>, because: &str) -> Assigned {
        Assigned {
            item,
            person: person.map(str::to_string),
            because: Some(because.into()),
        }
    }

    /// Whose each item ended up, by name, for readable assertions.
    fn whose(people: &[dto::Person], guesses: &[Option<Guess>]) -> Vec<Option<String>> {
        guesses
            .iter()
            .map(|guess| {
                let guess = guess.as_ref()?;
                let person = people.iter().find(|p| p.id == guess.person_id)?;
                Some(person.name.clone())
            })
            .collect()
    }

    #[test]
    fn guesses_line_up_with_the_items_they_were_made_about() {
        let people = pair();
        let guesses = resolve(
            &people,
            3,
            &[
                said(1, Some("Josh"), "beer is his"),
                said(2, None, "everyone eats bread"),
                said(3, Some("Ash"), "hers"),
            ],
        );
        assert_eq!(
            whose(&people, &guesses),
            [Some("Josh".into()), None, Some("Ash".into())]
        );
        assert_eq!(guesses[0].as_ref().unwrap().why, "beer is his");
    }

    /// A name nobody answers to is the model inventing somebody, and an item
    /// number that isn't on the receipt is it losing count. Neither is written.
    #[test]
    fn invented_names_and_numbers_are_thrown_away() {
        let people = pair();
        let guesses = resolve(
            &people,
            2,
            &[
                said(1, Some("Josh's mum"), "invented"),
                said(2, Some(""), "nobody"),
                said(9, Some("Ash"), "off the end"),
                said(0, Some("Ash"), "before the start"),
            ],
        );
        assert_eq!(whose(&people, &guesses), [None, None]);
    }

    /// Case and stray spaces are the model's, not a different person.
    #[test]
    fn a_name_is_matched_however_it_is_typed() {
        let people = pair();
        let guesses = resolve(&people, 1, &[said(1, Some("  josh "), "beer is his")]);
        assert_eq!(whose(&people, &guesses), [Some("Josh".into())]);
    }

    /// Every guess is shown on screen, so every guess needs something to show.
    #[test]
    fn a_guess_with_no_reason_still_says_where_it_came_from() {
        let people = pair();
        let bare = Assigned {
            item: 1,
            person: Some("Ash".into()),
            because: Some("   ".into()),
        };
        let guesses = resolve(&people, 1, &[bare]);
        assert_eq!(guesses[0].as_ref().unwrap().why, "from their description");
    }

    #[test]
    fn the_first_word_on_an_item_is_the_one_that_counts() {
        let people = pair();
        let guesses = resolve(
            &people,
            1,
            &[
                said(1, Some("Josh"), "first"),
                said(1, Some("Ash"), "second"),
            ],
        );
        assert_eq!(whose(&people, &guesses), [Some("Josh".into())]);
    }

    /// Without a description there's nothing to reason from, and asking the model
    /// would cost ten seconds an upload to be told nothing.
    #[test]
    fn nobody_described_means_nothing_to_ask() {
        assert!(pair().iter().all(dto::Person::described));
        assert!(!person(1, "Josh", None).described());
        assert!(!person(1, "Josh", Some("  ")).described());
    }

    /// Says whatever it was told to, and keeps the question so a test can check
    /// what it was asked.
    struct Fake {
        answer: Vec<Assigned>,
        asked: std::sync::Mutex<Vec<String>>,
    }

    impl Fake {
        fn saying(answer: Vec<Assigned>) -> Self {
            Self {
                answer,
                asked: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn asked(&self) -> Vec<String> {
            self.asked.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl ItemAssigner for Fake {
        async fn assign(
            &self,
            _merchant: &str,
            _people: &[dto::Person],
            items: &[&str],
        ) -> anyhow::Result<Vec<Assigned>> {
            *self.asked.lock().unwrap() = items.iter().map(|i| i.to_string()).collect();
            Ok(self.answer.clone())
        }
    }

    async fn memory_db() -> toasty::Db {
        crate::server::db::connect_url("sqlite::memory:")
            .await
            .unwrap()
    }

    /// Josh, who drinks the beer, and Ash, who doesn't eat meat.
    async fn people(db: &mut toasty::Db) -> (Uuid, Uuid) {
        let josh = toasty::create!(models::Person {
            name: "Josh",
            description: "drinks all the beer",
        })
        .exec(db)
        .await
        .unwrap();
        let ash = toasty::create!(models::Person {
            name: "Ash",
            description: "vegetarian",
        })
        .exec(db)
        .await
        .unwrap();
        (josh.id, ash.id)
    }

    /// Each item's description with whose it is and why, in receipt order.
    async fn items(
        db: &mut toasty::Db,
        receipt_id: Uuid,
    ) -> Vec<(String, Option<Uuid>, Option<String>)> {
        let receipt = models::Receipt::get_by_id(db, &receipt_id).await.unwrap();
        let mut items = receipt.line_items().exec(db).await.unwrap();
        items.sort_by_key(|item| item.position);
        items
            .into_iter()
            .map(|item| (item.description, item.person_id, item.guessed_why))
            .collect()
    }

    /// The whole point of the thing: beer to the one who drinks it, meat away from
    /// the one who doesn't, and hands off what a human already decided.
    #[tokio::test]
    async fn guesses_fill_in_around_what_a_human_has_decided() {
        let mut db = memory_db().await;
        let (_josh, ash) = people(&mut db).await;

        // Positions out of insertion order, since the numbers the model answers
        // with mean the receipt's order and nothing else.
        let receipt = toasty::create!(models::Receipt {
            purchased_on: jiff::civil::date(2026, 7, 20),
            merchant: "Costco",
            total: rust_decimal::Decimal::from(30),
            currency: "USD",
            image_path: "a.jpg",
            line_items: [
                { description: "MILK 2%", total: rust_decimal::Decimal::from(4), position: 2 },
                { description: "BUD LIGHT 12PK", total: rust_decimal::Decimal::from(18), position: 0 },
                // Ash's, by hand: nothing may touch this.
                { description: "GROUND BEEF", total: rust_decimal::Decimal::from(8), position: 1, person_id: ash },
            ],
        })
        .exec(&mut db)
        .await
        .unwrap();

        let fake = Fake::saying(vec![
            said(1, Some("Josh"), "he drinks the beer"),
            // Wrong, and it's a human's call anyway.
            said(2, Some("Josh"), "Ash is vegetarian"),
            Assigned {
                item: 3,
                person: None,
                because: None,
            },
        ]);

        let named = suggest(&mut db, &fake, receipt.id).await.unwrap();
        assert_eq!(named, 1);
        assert_eq!(
            fake.asked(),
            ["BUD LIGHT 12PK", "GROUND BEEF", "MILK 2%"],
            "asked in the receipt's own order"
        );

        let items = items(&mut db, receipt.id).await;
        assert_eq!(items[0].2.as_deref(), Some("he drinks the beer"));
        assert!(items[0].1.is_some(), "the beer went to somebody");
        assert_eq!(items[1].1, Some(ash), "left as the human had it");
        assert_eq!(items[1].2, None, "and still not a guess");
        assert_eq!(items[2].1, None, "nothing to say about milk");
    }

    /// Asking again after a description changes has to replace what it said last
    /// time, including taking a name back off.
    #[tokio::test]
    async fn a_second_guess_replaces_the_first() {
        let mut db = memory_db().await;
        let (josh, _ash) = people(&mut db).await;

        let receipt = toasty::create!(models::Receipt {
            purchased_on: jiff::civil::date(2026, 7, 20),
            merchant: "Costco",
            total: rust_decimal::Decimal::from(30),
            currency: "USD",
            image_path: "a.jpg",
            line_items: [
                { description: "BUD LIGHT 12PK", total: rust_decimal::Decimal::from(18), position: 0, person_id: josh, guessed_why: "he drinks the beer" },
                { description: "MILK 2%", total: rust_decimal::Decimal::from(4), position: 1 },
            ],
        })
        .exec(&mut db)
        .await
        .unwrap();

        // It's changed its mind about the beer and found something to say about
        // the milk.
        let fake = Fake::saying(vec![said(2, Some("Ash"), "she has the oat milk")]);
        assert_eq!(suggest(&mut db, &fake, receipt.id).await.unwrap(), 1);

        let items = items(&mut db, receipt.id).await;
        assert_eq!(items[0].1, None, "the old guess is gone, not left over");
        assert_eq!(items[0].2, None);
        assert_eq!(items[1].2.as_deref(), Some("she has the oat milk"));
    }

    /// Nobody described is most of the life of a fresh install, and asking anyway
    /// would cost ten seconds an upload.
    #[tokio::test]
    async fn a_receipt_with_nothing_to_go_on_never_asks() {
        let mut db = memory_db().await;
        toasty::create!(models::Person { name: "Josh" })
            .exec(&mut db)
            .await
            .unwrap();

        let receipt = toasty::create!(models::Receipt {
            purchased_on: jiff::civil::date(2026, 7, 20),
            merchant: "Costco",
            currency: "USD",
            image_path: "a.jpg",
            line_items: [{ description: "MILK 2%", total: rust_decimal::Decimal::from(4), position: 0 }],
        })
        .exec(&mut db)
        .await
        .unwrap();

        let fake = Fake::saying(vec![said(1, Some("Josh"), "why not")]);
        assert_eq!(suggest(&mut db, &fake, receipt.id).await.unwrap(), 0);
        assert!(fake.asked().is_empty(), "the model was asked anyway");
        assert_eq!(items(&mut db, receipt.id).await[0].1, None);
    }

    /// The question has to name everybody, described or not: elimination is half
    /// of what makes this work.
    #[test]
    fn the_question_names_everyone_and_numbers_the_items() {
        let asked = question("Costco", &pair(), &["BUD LIGHT 12PK", "GROUND BEEF"]);
        assert!(asked.contains("- Josh: drinks all the beer"), "{asked}");
        assert!(asked.contains("- Ash: vegetarian"), "{asked}");
        assert!(asked.contains("Receipt from Costco:"), "{asked}");
        assert!(asked.contains("1. BUD LIGHT 12PK"), "{asked}");
        assert!(asked.contains("2. GROUND BEEF"), "{asked}");

        let asked = question("Costco", &[person(1, "Guest", None)], &["MILK"]);
        assert!(asked.contains("- Guest: nothing said"), "{asked}");
    }
}
