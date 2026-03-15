// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Seeded Contacts E2E Tests
//!
//! Validates contact management with a realistic data set:
//! search (name + field), group filtering, scrolling/pagination,
//! and contact detail rendering through the AppEngine pipeline.
//!
//! Uses the same fake-contact seeding pattern as the `seed-contacts` binary.
//!
//! Feature: contacts_management.feature @contacts @search @filter @groups

use vauchi_core::contact_card::ContactCard;
use vauchi_core::ui::{AppEngine, AppScreen, Component, UserAction, WorkflowEngine};
use vauchi_core::{Contact, ContactField, FieldType, SymmetricKey, Vauchi};

/// Seed N fake contacts with groups into a Vauchi instance.
/// Returns group IDs as (family, friends, work).
fn seed_contacts(vauchi: &Vauchi, count: usize) -> (String, String, String) {
    let family = vauchi.create_group("Family").expect("create group");
    let friends = vauchi.create_group("Friends").expect("create group");
    let work = vauchi.create_group("Work").expect("create group");
    let family_id = family.id().to_string();
    let friends_id = friends.id().to_string();
    let work_id = work.id().to_string();

    let names = [
        "Abigale Schroeder",
        "Ahmed Nikolaus",
        "Andreanne Doyle",
        "Andres Raynor",
        "Augusta Heller",
        "Brady Koss",
        "Carmen Gutierrez",
        "Diana Patel",
        "Eduardo Reyes",
        "Fatima Okonkwo",
        "Gerhard Mueller",
        "Hiroshi Tanaka",
        "Ingrid Svensson",
        "Jorge Castillo",
        "Keiko Watanabe",
        "Liam O'Brien",
        "Maria Santos",
        "Nikolai Petrov",
        "Olga Ivanova",
        "Pedro Almeida",
        "Qian Wei",
        "Rosa Martinez",
        "Stefan Braun",
        "Tomoko Yamada",
        "Uma Krishnamurthy",
        "Viktor Novak",
        "Wendy Chang",
        "Xavier Dupont",
        "Yuki Mori",
        "Zara Al-Rashidi",
    ];

    for (i, name) in names.iter().take(count).enumerate() {
        let area = 200 + (i * 3) % 800;
        let num1 = 100 + (i * 7) % 900;
        let num2 = 1000 + (i * 13) % 9000;
        let phone = format!("+1-{}-{}-{}", area, num1, num2);
        let email = format!(
            "{}@example.com",
            name.to_lowercase().replace(' ', ".").replace('\'', "")
        );

        let mut card = ContactCard::new(name);
        card.add_field(ContactField::new(FieldType::Phone, "Mobile", &phone))
            .expect("add phone");
        card.add_field(ContactField::new(FieldType::Email, "Email", &email))
            .expect("add email");

        if i % 5 < 2 {
            card.add_field(ContactField::new(
                FieldType::Address,
                "Address",
                &format!("{} Main St, Springfield", 100 + i * 10),
            ))
            .expect("add address");
        }

        let mut pk = [0u8; 32];
        pk[0] = (i >> 8) as u8;
        pk[1] = (i & 0xFF) as u8;
        for (j, byte) in pk[2..].iter_mut().enumerate() {
            *byte = ((i * 7 + j) & 0xFF) as u8;
        }

        let contact = Contact::from_exchange(pk, card, SymmetricKey::generate());
        let cid = contact.id().to_string();
        vauchi.add_contact(contact).expect("add contact");

        let gid = match i % 10 {
            0..=2 => &family_id,
            3..=6 => &friends_id,
            _ => &work_id,
        };
        vauchi
            .add_contact_to_group(gid, &cid)
            .expect("add to group");
    }

    (family_id, friends_id, work_id)
}

/// Create AppEngine with identity and N seeded contacts.
fn create_seeded_engine(count: usize) -> AppEngine {
    let mut vauchi = Vauchi::in_memory().expect("vauchi");
    vauchi
        .create_identity("Test User")
        .expect("create identity");
    vauchi
        .add_own_field(ContactField::new(
            FieldType::Email,
            "Email",
            "test@vauchi.app",
        ))
        .expect("add own field");
    vauchi
        .add_own_field(ContactField::new(FieldType::Phone, "Mobile", "+1-555-0000"))
        .expect("add own field");

    seed_contacts(&vauchi, count);
    AppEngine::new(vauchi)
}

/// Extract contact names from the current screen's contact list component.
fn contact_names(engine: &dyn WorkflowEngine) -> Vec<String> {
    let screen = engine.current_screen();
    screen
        .components
        .iter()
        .find_map(|c| match c {
            Component::ContactList { contacts, .. } => {
                Some(contacts.iter().map(|c| c.name.clone()).collect())
            }
            _ => None,
        })
        .unwrap_or_default()
}

// ============================================================
// E2E: Contact list with seeded data
// Feature: contacts_management.feature @contacts @view
// ============================================================

/// @scenario: contacts_management:View all contacts with populated list
#[test]
fn e2e_seeded_contacts_all_visible_on_contacts_screen() {
    let mut engine = create_seeded_engine(10);
    engine.navigate_to(AppScreen::Contacts);

    let names = contact_names(&engine);
    assert_eq!(names.len(), 10, "all 10 seeded contacts should appear");

    // Verify alphabetical ordering
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "contacts should be in alphabetical order");
}

/// @scenario: contacts_management:Contact list shows subtitle from first field
#[test]
fn e2e_seeded_contacts_have_subtitle() {
    let mut engine = create_seeded_engine(5);
    engine.navigate_to(AppScreen::Contacts);

    let screen = engine.current_screen();
    let contacts = screen
        .components
        .iter()
        .find_map(|c| match c {
            Component::ContactList { contacts, .. } => Some(contacts),
            _ => None,
        })
        .expect("contacts component");

    for contact in contacts {
        assert!(
            contact.subtitle.is_some(),
            "contact '{}' should have subtitle (first field value)",
            contact.name
        );
    }
}

/// @scenario: contacts_management:Contact list shows searchable fields
#[test]
fn e2e_seeded_contacts_have_searchable_fields() {
    let mut engine = create_seeded_engine(5);
    engine.navigate_to(AppScreen::Contacts);

    let screen = engine.current_screen();
    let contacts = screen
        .components
        .iter()
        .find_map(|c| match c {
            Component::ContactList { contacts, .. } => Some(contacts),
            _ => None,
        })
        .expect("contacts component");

    for contact in contacts {
        assert!(
            !contact.searchable_fields.is_empty(),
            "contact '{}' should have searchable fields (phone, email)",
            contact.name
        );
        // Each contact should have at least phone + email
        assert!(
            contact.searchable_fields.len() >= 2,
            "contact '{}' should have at least 2 searchable fields, got {}",
            contact.name,
            contact.searchable_fields.len()
        );
    }
}

// ============================================================
// E2E: Search by name
// Feature: contacts_management.feature @contacts @search
// ============================================================

/// @scenario: contacts_management:Search contacts by name
#[test]
fn e2e_search_by_name_filters_correctly() {
    let mut engine = create_seeded_engine(15);
    engine.navigate_to(AppScreen::Contacts);

    let _ = engine.handle_action(UserAction::SearchChanged {
        component_id: "contacts".into(),
        query: "Ahmed".into(),
    });

    let names = contact_names(&engine);
    assert_eq!(names.len(), 1, "search 'Ahmed' should find exactly 1 match");
    assert_eq!(names[0], "Ahmed Nikolaus");
}

/// @scenario: contacts_management:Search contacts partial match
#[test]
fn e2e_search_partial_name_matches() {
    let mut engine = create_seeded_engine(15);
    engine.navigate_to(AppScreen::Contacts);

    let _ = engine.handle_action(UserAction::SearchChanged {
        component_id: "contacts".into(),
        query: "an".into(),
    });

    let names = contact_names(&engine);
    // "an" matches names containing "an" OR fields containing "an"
    assert!(
        names.len() >= 2,
        "partial search 'an' should match multiple contacts, got {}",
        names.len()
    );
}

/// @scenario: contacts_management:Search contacts no results
#[test]
fn e2e_search_no_match_returns_empty() {
    let mut engine = create_seeded_engine(10);
    engine.navigate_to(AppScreen::Contacts);

    let _ = engine.handle_action(UserAction::SearchChanged {
        component_id: "contacts".into(),
        query: "zzzznonexistent".into(),
    });

    let names = contact_names(&engine);
    assert!(names.is_empty(), "nonsense query should return no results");
}

// ============================================================
// E2E: Search by field value (email, phone)
// Feature: contacts_management.feature @contacts @search
// ============================================================

/// @scenario: contacts_management:Search contacts by email
#[test]
fn e2e_search_by_email_matches() {
    let mut engine = create_seeded_engine(10);
    engine.navigate_to(AppScreen::Contacts);

    let _ = engine.handle_action(UserAction::SearchChanged {
        component_id: "contacts".into(),
        query: "carmen".into(),
    });

    let names = contact_names(&engine);
    assert_eq!(
        names.len(),
        1,
        "search 'carmen' should match Carmen Gutierrez via name or email"
    );
    assert_eq!(names[0], "Carmen Gutierrez");
}

/// @scenario: contacts_management:Search contacts by phone number
#[test]
fn e2e_search_by_phone_matches() {
    let mut engine = create_seeded_engine(10);
    engine.navigate_to(AppScreen::Contacts);

    let _ = engine.handle_action(UserAction::SearchChanged {
        component_id: "contacts".into(),
        query: "+1-212".into(),
    });

    let names = contact_names(&engine);
    assert_eq!(
        names.len(),
        1,
        "search '+1-212' should match Augusta Heller's phone"
    );
    assert_eq!(names[0], "Augusta Heller");
}

// ============================================================
// E2E: Group filtering
// Feature: contacts_management.feature @contacts @groups @filter
// ============================================================

/// @scenario: contacts_management:Filter contacts by group
#[test]
fn e2e_group_filter_shows_only_group_members() {
    let mut engine = create_seeded_engine(10);
    engine.navigate_to(AppScreen::Contacts);

    // Find the Family group filter action
    let screen = engine.current_screen();
    let family_action = screen
        .actions
        .iter()
        .find(|a| a.id.starts_with("filter_group:") && a.label == "Family")
        .expect("Family group filter action should exist");
    let family_action_id = family_action.id.clone();

    let _ = engine.handle_action(UserAction::ActionPressed {
        action_id: family_action_id,
    });

    let names = contact_names(&engine);
    // With 10 contacts and i%10: 0..=2 → Family, we get indices 0,1,2 = 3 contacts
    assert_eq!(
        names.len(),
        3,
        "Family filter should show 3 members, got {} ({:?})",
        names.len(),
        names
    );
}

/// @scenario: contacts_management:Clear group filter shows all contacts
#[test]
fn e2e_group_filter_clear_shows_all() {
    let mut engine = create_seeded_engine(10);
    engine.navigate_to(AppScreen::Contacts);

    // Apply filter
    let screen = engine.current_screen();
    let family_action_id = screen
        .actions
        .iter()
        .find(|a| a.id.starts_with("filter_group:") && a.label == "Family")
        .map(|a| a.id.clone())
        .expect("Family filter action");

    let _ = engine.handle_action(UserAction::ActionPressed {
        action_id: family_action_id,
    });
    assert_eq!(contact_names(&engine).len(), 3);

    // Clear filter
    let _ = engine.handle_action(UserAction::ActionPressed {
        action_id: "filter_group_clear".into(),
    });

    let names = contact_names(&engine);
    assert_eq!(
        names.len(),
        10,
        "clearing filter should show all 10 contacts"
    );
}

/// @scenario: contacts_management:Group filter combines with search
#[test]
fn e2e_group_filter_combined_with_search() {
    let mut engine = create_seeded_engine(15);
    engine.navigate_to(AppScreen::Contacts);

    // Apply Friends filter (indices 3..=6 → 4 per 10)
    let screen = engine.current_screen();
    let friends_action_id = screen
        .actions
        .iter()
        .find(|a| a.id.starts_with("filter_group:") && a.label == "Friends")
        .map(|a| a.id.clone())
        .expect("Friends filter action");

    let _ = engine.handle_action(UserAction::ActionPressed {
        action_id: friends_action_id,
    });

    let friends_names = contact_names(&engine);
    assert!(
        friends_names.len() > 1,
        "Friends group should have multiple members"
    );

    // Now search within Friends
    let _ = engine.handle_action(UserAction::SearchChanged {
        component_id: "contacts".into(),
        query: friends_names[0].split_whitespace().next().unwrap().into(),
    });

    let filtered = contact_names(&engine);
    assert!(
        filtered.len() <= friends_names.len(),
        "search within group should narrow results"
    );
}

// ============================================================
// E2E: Group actions in screen model
// Feature: contacts_management.feature @contacts @groups
// ============================================================

/// @scenario: contacts_management:Groups appear as filter actions
#[test]
fn e2e_groups_appear_as_screen_actions() {
    let mut engine = create_seeded_engine(5);
    engine.navigate_to(AppScreen::Contacts);

    let screen = engine.current_screen();
    let group_actions: Vec<&str> = screen
        .actions
        .iter()
        .filter(|a| a.id.starts_with("filter_group:"))
        .map(|a| a.label.as_str())
        .collect();

    assert!(
        group_actions.contains(&"Family"),
        "Family group action should exist"
    );
    assert!(
        group_actions.contains(&"Friends"),
        "Friends group action should exist"
    );
    assert!(
        group_actions.contains(&"Work"),
        "Work group action should exist"
    );
}

// ============================================================
// E2E: Large dataset stress
// Feature: contacts_management.feature @contacts @scroll
// ============================================================

/// @scenario: contacts_management:Scroll through large contact list
#[test]
fn e2e_large_contact_list_renders() {
    let mut engine = create_seeded_engine(30);
    engine.navigate_to(AppScreen::Contacts);

    let names = contact_names(&engine);
    assert_eq!(names.len(), 30, "all 30 contacts should be present");

    // Verify first and last
    assert_eq!(names[0], "Abigale Schroeder");
    assert_eq!(names[29], "Zara Al-Rashidi");
}

/// @scenario: contacts_management:Search scales with many contacts
#[test]
fn e2e_search_with_30_contacts() {
    let mut engine = create_seeded_engine(30);
    engine.navigate_to(AppScreen::Contacts);

    let _ = engine.handle_action(UserAction::SearchChanged {
        component_id: "contacts".into(),
        query: "Zara".into(),
    });

    let names = contact_names(&engine);
    assert_eq!(names.len(), 1);
    assert_eq!(names[0], "Zara Al-Rashidi");
}

// ============================================================
// E2E: Contact detail with seeded fields
// Feature: contacts_management.feature @contacts @view
// ============================================================

/// @scenario: contacts_management:View contact detail shows all fields
#[test]
fn e2e_contact_detail_shows_seeded_fields() {
    let mut engine = create_seeded_engine(5);
    engine.navigate_to(AppScreen::Contacts);

    // Get first contact's ID
    let screen = engine.current_screen();
    let first_contact_id = screen
        .components
        .iter()
        .find_map(|c| match c {
            Component::ContactList { contacts, .. } => contacts.first().map(|c| c.id.clone()),
            _ => None,
        })
        .expect("first contact ID");

    // Navigate to detail
    engine.navigate_to(AppScreen::ContactDetail {
        contact_id: first_contact_id,
    });

    let detail_screen = engine.current_screen();
    let field_count = detail_screen
        .components
        .iter()
        .filter(|c| matches!(c, Component::FieldList { .. }))
        .count();

    // Contact detail should have a FieldList component with the contact's fields
    assert!(
        field_count >= 1,
        "contact detail should show a FieldList component, got {}",
        field_count
    );
}
