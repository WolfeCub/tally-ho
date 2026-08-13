//! The people a line item can be charged to.

use crate::server::{mappers, models};
use crate::shared::dto;

/// Everyone a line item can be charged to, by name.
pub async fn list(db: &mut toasty::Db) -> toasty::Result<Vec<dto::Person>> {
    Ok(models::Person::all()
        .order_by(models::Person::fields().name().asc())
        .exec(db)
        .await?
        .iter()
        .map(mappers::to_dto_person)
        .collect())
}

/// A person as the settings screen left them, already parsed.
pub struct Save {
    /// `None` for someone the human added.
    pub id: Option<uuid::Uuid>,
    pub name: String,
    pub description: Option<String>,
}

/// Writes the settings screen in one go: everyone it ended up with, as a
/// complete replacement for whoever is stored.
///
/// One transaction — a half-applied save could leave items charged to somebody
/// who was meant to be gone.
pub async fn save(db: &mut toasty::Db, people: Vec<Save>) -> toasty::Result<()> {
    let mut tx = db.transaction().await?;

    let mut existing: std::collections::HashMap<_, _> = models::Person::all()
        .exec(&mut tx)
        .await?
        .into_iter()
        .map(|person| (person.id, person))
        .collect();

    for person in people {
        // Removing as we go, so what's left over is who the human removed. An
        // id that isn't there any more falls through to a create.
        match person.id.and_then(|id| existing.remove(&id)) {
            Some(mut row) => {
                toasty::update!(row {
                    name: person.name,
                    description: person.description,
                })
                .exec(&mut tx)
                .await?;
            }
            None => {
                toasty::create!(models::Person {
                    name: person.name,
                    description: person.description,
                })
                .exec(&mut tx)
                .await?;
            }
        }
    }

    for person in existing.into_values() {
        // Nothing cascades, and an item pointing at somebody who isn't there
        // would be neither assigned nor unassigned — it would drop out of both
        // halves of the split.
        for mut item in
            models::LineItem::filter(models::LineItem::fields().person_id().eq(person.id))
                .exec(&mut tx)
                .await?
        {
            toasty::update!(item {
                person_id: None,
                guessed_why: None,
            })
            .exec(&mut tx)
            .await?;
        }
        person.delete().exec(&mut tx).await?;
    }

    tx.commit().await
}

#[cfg(test)]
mod tests {
    use crate::server::models::Person;
    use crate::server::testing::memory_db;

    /// One save has to rename, add and remove people at once — the settings
    /// screen sends the list it ended up with, not a diff.
    #[tokio::test]
    async fn saving_people_replaces_the_whole_list() {
        let mut db = memory_db().await;

        let josh = toasty::create!(Person { name: "Josh" })
            .exec(&mut db)
            .await
            .unwrap();
        let typo = toasty::create!(Person { name: "Asj" })
            .exec(&mut db)
            .await
            .unwrap();

        super::save(
            &mut db,
            vec![
                // Untouched.
                super::Save {
                    id: Some(josh.id),
                    name: "Josh".into(),
                    description: None,
                },
                // Corrected, and described.
                super::Save {
                    id: Some(typo.id),
                    name: "Ash".into(),
                    description: Some("the other card".into()),
                },
                // Added on screen.
                super::Save {
                    id: None,
                    name: "Guest".into(),
                    description: None,
                },
            ],
        )
        .await
        .unwrap();

        let mut people = Person::all().exec(&mut db).await.unwrap();
        people.sort_by(|a, b| a.name.cmp(&b.name));
        let got: Vec<_> = people
            .iter()
            .map(|p| (p.name.as_str(), p.description.as_deref()))
            .collect();
        assert_eq!(
            got,
            [
                ("Ash", Some("the other card")),
                ("Guest", None),
                ("Josh", None),
            ]
        );
        // Renaming edits the row rather than replacing it, so anything charged
        // to them stays charged to them.
        assert_eq!(people[0].id, typo.id);
    }
}
