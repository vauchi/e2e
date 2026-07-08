// SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Seed a Vauchi instance with fake contacts for testing.
//!
//! Usage:
//!   cargo run --bin seed-contacts --features seed-contacts -- [--count N] [--seed S]
//!
//! Generates N fake contacts (default: 50) with realistic names,
//! phone numbers, emails, and addresses using the fake-rs crate.
//! Useful for testing backup load, scrolling, search, group filter, etc.

use fake::Fake;
use fake::faker::address::en::*;
use fake::faker::internet::en::*;
use fake::faker::name::en::*;
use rand::SeedableRng;
use rand::rngs::StdRng;
use vauchi_core::Vauchi;
use vauchi_core::contact::Contact;
use vauchi_core::contact_card::{ContactCard, ContactField, FieldType};
use vauchi_core::crypto::SymmetricKey;

/// Names that stress rendering: emoji, RTL, CJK, combining marks,
/// very long, and whitespace-adversarial.
const ADVERSARIAL_NAMES: &[&str] = &[
    "🦄🎉 Emoji Person 🚀",
    "محمد الرشيد",
    "山田太郎",
    "Z̴̢̛a̷l̸g̵o̶ ̷N̸a̵m̶e̷",
    "Anna-Maria Charlotte Elisabeth von Hohenberg-Wittgenstein zu Sayn",
    "  Leading Spaces",
];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let count: usize = parse_arg(&args, "--count").unwrap_or(50);
    let seed: u64 = parse_arg(&args, "--seed").unwrap_or(42);
    let own_fields: usize = parse_arg(&args, "--fields").unwrap_or(0);
    let backup_path: Option<String> = parse_arg(&args, "--backup");
    let backup_password: String =
        parse_arg(&args, "--backup-password").unwrap_or_else(|| "test123".to_string());
    let display_name: String =
        parse_arg(&args, "--name").unwrap_or_else(|| "Test User".to_string());

    println!("Seeding {} fake contacts (seed={})", count, seed);

    let mut vauchi = Vauchi::in_memory().expect("Failed to create Vauchi");

    vauchi
        .create_identity(&display_name)
        .expect("Failed to create identity");

    vauchi
        .add_own_field(ContactField::new(
            FieldType::Email,
            "Email",
            "test@vauchi.app",
            0,
        ))
        .expect("Failed to add own field");
    vauchi
        .add_own_field(ContactField::new(
            FieldType::Phone,
            "Mobile",
            "+1-555-0000",
            0,
        ))
        .expect("Failed to add own field");

    let family = vauchi.create_group("Family").expect("create group");
    let friends = vauchi.create_group("Friends").expect("create group");
    let work = vauchi.create_group("Work").expect("create group");
    let group_ids = [
        family.id().to_string(),
        friends.id().to_string(),
        work.id().to_string(),
    ];

    if own_fields > 0 {
        let mut added = 0usize;
        let mut first_error: Option<String> = None;
        for i in 0..own_fields {
            let (ft, label, value) = match i % 4 {
                0 => (
                    FieldType::Email,
                    format!("Email {i}"),
                    format!("me+{i}@vauchi.app"),
                ),
                1 => (
                    FieldType::Phone,
                    format!("Phone {i}"),
                    format!("+41-79-{:03}-{:04}", i % 1000, 1000 + i % 9000),
                ),
                2 => (
                    FieldType::Website,
                    format!("Social {i}"),
                    format!("https://social{i}.example.com/me"),
                ),
                _ => (
                    FieldType::Address,
                    format!("Address {i}"),
                    format!("{i} Test Street, Zurich"),
                ),
            };
            match vauchi.add_own_field(ContactField::new(ft, &label, &value, i as u64)) {
                Ok(_) => added += 1,
                Err(e) => {
                    first_error.get_or_insert_with(|| format!("field {i}: {e}"));
                }
            }
        }
        println!(
            "Own fields: {}/{} added{}",
            added,
            own_fields,
            first_error
                .map(|e| format!(" (first error: {e})"))
                .unwrap_or_default()
        );
    }

    let mut rng = StdRng::seed_from_u64(seed);

    for i in 0..count {
        let name: String = if i > 0 && i % 100 == 0 {
            format!(
                "{} {}",
                ADVERSARIAL_NAMES[(i / 100) % ADVERSARIAL_NAMES.len()],
                i / 100
            )
        } else {
            Name().fake_with_rng(&mut rng)
        };
        // Generate a phone number in valid format (digits, dashes, plus only)
        let area: u16 = (200 + (i * 3) % 800) as u16;
        let num1: u16 = (100 + (i * 7) % 900) as u16;
        let num2: u16 = (1000 + (i * 13) % 9000) as u16;
        let phone = format!("+1-{}-{}-{}", area, num1, num2);
        let email: String = FreeEmail().fake_with_rng(&mut rng);

        let mut card = ContactCard::new(&name);
        card.add_field(ContactField::new(FieldType::Phone, "Mobile", &phone, 0))
            .expect("add phone");
        card.add_field(ContactField::new(FieldType::Email, "Email", &email, 0))
            .expect("add email");

        // Address for ~40% of contacts
        if i % 5 < 2 {
            let street: String = StreetName().fake_with_rng(&mut rng);
            let city: String = CityName().fake_with_rng(&mut rng);
            card.add_field(ContactField::new(
                FieldType::Address,
                "Address",
                &format!("{}, {}", street, city),
                0,
            ))
            .expect("add address");
        }

        // Website for ~20%
        if i % 5 == 0 {
            let domain: String = DomainSuffix().fake_with_rng(&mut rng);
            card.add_field(ContactField::new(
                FieldType::Website,
                "Website",
                &format!(
                    "https://{}.{}",
                    name.to_lowercase().replace(' ', ""),
                    domain
                ),
                0,
            ))
            .expect("add website");
        }

        // Deterministic fake public key
        let mut pk = [0u8; 32];
        pk[0] = (i >> 8) as u8;
        pk[1] = (i & 0xFF) as u8;
        for (j, byte) in pk[2..].iter_mut().enumerate() {
            *byte = ((i * 7 + j) & 0xFF) as u8;
        }

        let contact = Contact::from_exchange(
            pk,
            card,
            SymmetricKey::generate(),
            vauchi_core::clock::SystemClock::shared().unix_seconds(),
        );
        let cid = contact.id().to_string();
        vauchi.add_contact(contact).expect("add contact");

        // Assign to group: ~30% Family, ~40% Friends, ~30% Work
        let gid = match i % 10 {
            0..=2 => &group_ids[0],
            3..=6 => &group_ids[1],
            _ => &group_ids[2],
        };
        vauchi
            .add_contact_to_group(gid, &cid)
            .expect("add to group");

        if (i + 1) % 10 == 0 {
            println!("  Created {}/{} contacts...", i + 1, count);
        }
    }

    let contacts = vauchi.list_contacts().expect("list contacts");
    println!("\nDone! {} contacts seeded and verified.", contacts.len());

    let groups = vauchi.list_groups().expect("list groups");
    for g in &groups {
        let member_count = contacts
            .iter()
            .filter(|c| g.contains_contact(c.id()))
            .count();
        println!("  Group '{}': {} members", g.name(), member_count);
    }

    // Print sample data for verification
    println!("\nSample contacts:");
    for c in contacts.iter().take(5) {
        let fields: Vec<String> = c
            .card()
            .fields()
            .iter()
            .map(|f| format!("{}={}", f.label(), f.value()))
            .collect();
        println!("  {} [{}]", c.display_name(), fields.join(", "));
    }

    if let Some(path) = backup_path {
        println!("\nExporting full backup (password: {backup_password})...");
        let started = std::time::Instant::now();
        let backup_hex = vauchi
            .export_full_backup(&backup_password)
            .expect("export full backup");
        std::fs::write(&path, backup_hex.as_bytes()).expect("write backup file");
        println!(
            "Backup written to {} ({} bytes, {:.1}s)",
            path,
            backup_hex.len(),
            started.elapsed().as_secs_f32()
        );
    }
}

fn parse_arg<T: std::str::FromStr>(args: &[String], flag: &str) -> Option<T> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
}
