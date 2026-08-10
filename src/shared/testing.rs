//! Fixtures for the tests in this module. Never compiled into the binary.

use rust_decimal::Decimal;
use std::str::FromStr;
use uuid::Uuid;

use crate::shared::dto::{
    Charge, ExtractionStatus, ExtractionStatus::Done, LineItem, Matched, Person, Resolution,
    Statement,
};
use crate::shared::reconcile::split_charge;

pub fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

pub fn person(n: u128, name: &str) -> Person {
    Person {
        id: Uuid::from_u128(n),
        name: name.into(),
        description: None,
    }
}

/// Josh and Ash, who everything gets split between.
pub fn pair() -> Vec<Person> {
    vec![person(1, "Josh"), person(2, "Ash")]
}

pub fn item(total: &str) -> LineItem {
    LineItem {
        id: Uuid::nil(),
        description: "x".into(),
        quantity: None,
        unit_price: None,
        total: dec(total),
        position: 0,
        edited: false,
        person_id: None,
    }
}

/// A line item charged to one of [`pair`], or to nobody.
pub fn charged_to(whose: Option<u128>, total: &str) -> LineItem {
    LineItem {
        person_id: whose.map(Uuid::from_u128),
        ..item(total)
    }
}

pub fn matched(merchant: &str) -> Matched {
    Matched {
        receipt_id: Uuid::nil(),
        merchant: merchant.into(),
        purchased_on: jiff::civil::date(2026, 7, 1),
        total: Some(dec("10.00")),
        status: Done,
        reviewed: true,
        problems: Vec::new(),
    }
}

pub fn charge(description: &str, amount: &str, resolution: Resolution) -> Charge {
    let amount = dec(amount);
    Charge {
        id: Uuid::nil(),
        charged_on: jiff::civil::date(2026, 7, 4),
        description: description.into(),
        amount,
        split: split_charge(amount, "USD", &[], &pair()),
        resolution,
        suggestions: Vec::new(),
    }
}

pub fn statement(charges: Vec<Charge>) -> Statement {
    Statement {
        id: Uuid::nil(),
        label: "july.csv".into(),
        currency: "USD".into(),
        begins_on: jiff::civil::date(2026, 7, 1),
        ends_on: jiff::civil::date(2026, 7, 31),
        charges,
        people: pair(),
    }
}

/// A receipt with nothing on it, for tests that fill in only what they check.
pub fn receipt(total: Option<&str>, items: Vec<LineItem>) -> crate::shared::dto::Receipt {
    crate::shared::dto::Receipt {
        id: Uuid::nil(),
        purchased_on: jiff::civil::date(2026, 7, 1),
        merchant: "M".into(),
        subtotal: None,
        tax: None,
        total: total.map(dec),
        currency: "USD".into(),
        status: ExtractionStatus::Done,
        extraction_error: None,
        reviewed: false,
        line_items: items,
    }
}
