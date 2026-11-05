//! Groups relay sites into regions for the dropdown and the log report. Pure.
//!
//! Sites are classified from a table of codes because Valve's `geo` field is unreliable: Datapacket
//! sites have latitude and longitude swapped. A code the table doesn't know still gets a section.

pub const AFRICA: &str = "Africa";
pub const ASIA: &str = "Asia";
pub const EUROPE: &str = "Europe";
pub const MIDDLE_EAST: &str = "Middle East";
pub const NORTH_AMERICA: &str = "North America";
pub const OCEANIA: &str = "Oceania";
pub const SOUTH_AMERICA: &str = "South America";
pub const CHINA: &str = "China";
pub const OTHER: &str = "Other";

/// The order sections appear in. China stays near the end because its relays serve players there.
pub const ORDER: [&str; 9] = [
    AFRICA,
    ASIA,
    EUROPE,
    MIDDLE_EAST,
    NORTH_AMERICA,
    OCEANIA,
    SOUTH_AMERICA,
    CHINA,
    OTHER,
];

const SITES: &[(&str, &[&str])] = &[
    (AFRICA, &["jnb"]),
    (ASIA, &["bom2", "hkg", "maa2", "seo", "sgp", "tyo"]),
    (
        EUROPE,
        &[
            "ams", "dfra", "dlhr", "dvie", "fra", "lhr", "mad", "par", "sto", "sto2", "vie", "waw",
        ],
    ),
    (MIDDLE_EAST, &["dfjr", "dxb"]),
    (NORTH_AMERICA, &["atl", "dfw", "iad", "lax", "ord", "sea"]),
    (OCEANIA, &["gum", "syd"]),
    (SOUTH_AMERICA, &["eze", "gru", "lim", "scl"]),
    (
        CHINA,
        &[
            "ctum", "ctut", "ctuu", "pekm", "pekt", "peku", "pvgm", "pvgt", "pvgu", "tgdm", "tgdt",
            "tgdu",
        ],
    ),
];

/// The region a site belongs to. `desc` is only consulted for codes the table doesn't list.
pub fn of(code: &str, desc: &str) -> &'static str {
    SITES
        .iter()
        .find(|(_, codes)| codes.contains(&code))
        .map_or_else(
            || {
                if desc.contains("(China)") {
                    CHINA
                } else {
                    OTHER
                }
            },
            |(region, _)| region,
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_codes_use_the_table_whatever_the_name_says() {
        assert_eq!(of("jnb", "Johannesburg (South Africa)"), AFRICA);
        assert_eq!(of("dfra", "Datapacket Frankfurt (Germany)"), EUROPE);
        assert_eq!(of("gum", "Guam"), OCEANIA);
        assert_eq!(of("sgp", ""), ASIA);
        assert_eq!(of("dxb", "misleading (China)"), MIDDLE_EAST);
    }

    #[test]
    fn unknown_codes_still_get_a_section() {
        assert_eq!(of("newc", "Alibaba Cloud Somewhere (China)"), CHINA);
        assert_eq!(of("newx", "Somewhere (Atlantis)"), OTHER);
    }

    #[test]
    fn every_region_in_the_table_is_in_the_display_order() {
        for (region, _) in SITES {
            assert!(ORDER.contains(region), "{region} missing from ORDER");
        }
    }
}
