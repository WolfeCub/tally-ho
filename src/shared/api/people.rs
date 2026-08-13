//! Settings: the people a line item can be charged to.

use leptos::prelude::*;

use crate::shared::dto;

// Server-only, so these are behind a `cfg` rather than plain imports.
#[cfg(feature = "ssr")]
use {super::support::db, crate::server::queries::people, anyhow::Context as _};

/// Everyone a line item can be charged to, by name.
#[server]
pub async fn list_people() -> Result<Vec<dto::Person>, ServerFnError> {
    people::list(&mut db())
        .await
        .context("could not load people")
        .map_err(ServerFnError::new)
}

/// Applies the settings screen: everyone it ended up with, in one write.
///
/// Anybody missing from the list is removed, and whatever was charged to them
/// goes back to unassigned.
#[server]
pub async fn save_people(people: Vec<dto::PersonSave>) -> Result<(), ServerFnError> {
    use crate::server::queries::people::Save;

    let mut parsed = Vec::with_capacity(people.len());
    for person in people {
        let name = person.name.trim();
        // A row added and then left alone is dropped rather than complained about.
        if name.is_empty() && person.description.trim().is_empty() {
            continue;
        }
        if name.is_empty() {
            return Err(ServerFnError::new("a person needs a name"));
        }
        parsed.push(Save {
            id: person.id,
            name: name.to_string(),
            // A blank box means no description, not an empty one.
            description: {
                let described = person.description.trim();
                (!described.is_empty()).then(|| described.to_string())
            },
        });
    }

    people::save(&mut db(), parsed)
        .await
        .context("could not save people")
        .map_err(ServerFnError::new)
}
