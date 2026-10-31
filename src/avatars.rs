//! Server-generated default portraits. Only fixed SVG templates and numeric hash
//! choices enter the image; no user text, URLs, scripts or external assets do.
use anyhow::Result;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

pub fn ensure_defaults(db: &Connection) -> Result<()> {
    let missing = db.prepare("SELECT a.id,a.handle FROM board_actors a LEFT JOIN board_default_avatars d ON d.actor=a.id WHERE d.actor IS NULL")?
        .query_map([], |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, alias) in missing {
        let image = creature(&alias);
        db.execute(
            "INSERT OR IGNORE INTO board_default_avatars(actor,data,revision) VALUES(?1,?2,?3)",
            params![id, image.as_bytes(), crate::security::digest(&image)],
        )?;
    }
    Ok(())
}

fn creature(alias: &str) -> String {
    let hash = Sha256::digest(format!("space-creatures-v1:{}", alias.to_lowercase()));
    let palettes = [
        ("#f0dcc6", "#e99c65", "#a55340", "#f6c86a"),
        ("#d8e5d5", "#81b49d", "#3b776b", "#eed578"),
        ("#e0dcf1", "#a99bd3", "#655884", "#efacaa"),
        ("#d5e7ed", "#7cb8d2", "#467a9d", "#efd181"),
        ("#f0dadd", "#d892a8", "#995773", "#f1c387"),
        ("#f0e8c9", "#d9bd6b", "#968247", "#a5c9a5"),
        ("#d9e6e8", "#8ebec0", "#507e8d", "#e9b68a"),
        ("#e7dcd3", "#bca38c", "#776c68", "#b9cf8b"),
    ];
    let (background, skin, shade, accent) = palettes[hash[0] as usize % palettes.len()];
    let ink = "#263844";
    let head = match hash[1] % 5 {
        0 => "M21 57C21 35 30 23 48 23S75 35 75 57V82H21Z",
        1 => "M18 56C18 35 29 26 48 26S78 35 78 56C78 76 65 84 48 84S18 76 18 56Z",
        2 => "M24 37Q24 24 38 24H58Q72 24 72 38L78 75Q79 84 66 84H30Q17 84 18 75Z",
        3 => "M20 50Q17 30 35 29Q48 16 61 29Q79 30 76 50L73 81H23Z",
        _ => "M19 55Q16 40 27 37Q28 24 40 28Q49 17 58 28Q73 24 72 38Q83 43 77 58L73 83H23Z",
    };
    let ears = match hash[2] % 5 {
        0 => format!(
            "<path d='M28 34Q15 7 26 9Q36 13 38 29M58 29Q60 13 70 9Q81 7 68 34' fill='{skin}'/><path d='M29 27L26 17M67 27L70 17' stroke='{shade}'/>"
        ),
        1 => {
            format!("<path d='M28 35L15 15Q33 15 38 29M58 29Q63 15 81 15L68 35' fill='{accent}'/>")
        }
        2 => format!(
            "<path d='M39 28L34 12M57 28L62 12' stroke='{shade}'/><circle cx='33' cy='11' r='5' fill='{accent}'/><circle cx='63' cy='11' r='5' fill='{accent}'/>"
        ),
        3 => format!(
            "<path d='M29 37Q9 20 9 35Q10 49 25 47M67 37Q87 20 87 35Q86 49 71 47' fill='{accent}'/>"
        ),
        _ => format!(
            "<path d='M38 29Q31 17 41 12Q39 23 49 26M58 29Q67 20 61 13Q77 19 66 36' fill='{accent}'/>"
        ),
    };
    let eyes = match hash[3] % 5 {
        0 => format!(
            "<ellipse cx='36' cy='47' rx='9' ry='10' fill='#fff8e9'/><ellipse cx='60' cy='47' rx='9' ry='10' fill='#fff8e9'/><circle cx='38' cy='48' r='4' fill='{ink}'/><circle cx='62' cy='48' r='4' fill='{ink}'/>"
        ),
        1 => format!(
            "<ellipse cx='48' cy='46' rx='16' ry='14' fill='#fff8e9'/><circle cx='50' cy='47' r='7' fill='{ink}'/><circle cx='52' cy='44' r='2' fill='white'/>"
        ),
        2 => format!(
            "<circle cx='35' cy='47' r='8' fill='#fff8e9'/><circle cx='61' cy='47' r='8' fill='#fff8e9'/><circle cx='35' cy='49' r='4' fill='{ink}'/><circle cx='61' cy='49' r='4' fill='{ink}'/><path d='M29 34L39 32M57 32L67 34' stroke='{shade}'/>"
        ),
        3 => format!(
            "<path d='M29 48Q35 39 41 48M55 48Q61 39 67 48' stroke='{ink}' stroke-width='4' fill='none'/>"
        ),
        _ => format!(
            "<circle cx='35' cy='47' r='9' fill='#fff8e9'/><circle cx='35' cy='48' r='4' fill='{ink}'/><path d='M55 48Q61 42 67 48' stroke='{ink}' stroke-width='4' fill='none'/>"
        ),
    };
    let mouth = match hash[4] % 4 {
        0 => format!("<path d='M39 63Q48 73 57 63' fill='none' stroke='{ink}' stroke-width='3'/>"),
        1 => format!(
            "<path d='M40 62H56Q55 73 48 73Q41 73 40 62' fill='{ink}'/><path d='M45 63H51V67H45Z' fill='#fff8e9' stroke='none'/>"
        ),
        2 => format!("<ellipse cx='48' cy='65' rx='4' ry='5' fill='{ink}'/>"),
        _ => format!(
            "<path d='M38 64Q43 68 48 63Q53 68 58 64' fill='none' stroke='{ink}' stroke-width='3'/>"
        ),
    };
    let cheeks = if hash[5] % 2 == 0 {
        format!(
            "<ellipse cx='27' cy='59' rx='5' ry='3' fill='{accent}'/><ellipse cx='69' cy='59' rx='5' ry='3' fill='{accent}'/>"
        )
    } else {
        format!("<path d='M26 58L29 59M67 59L70 58' stroke='{shade}' stroke-width='3'/>")
    };
    let chest = match hash[6] % 3 {
        0 => format!("<path d='M42 83L48 77L54 83L48 89Z' fill='{accent}'/>"),
        1 => format!("<circle cx='48' cy='83' r='5' fill='{accent}'/>"),
        _ => {
            format!("<path d='M40 81Q48 87 56 81' stroke='{accent}' stroke-width='5' fill='none'/>")
        }
    };
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='96' height='96' viewBox='0 0 96 96'><rect width='96' height='96' fill='{background}'/><circle cx='48' cy='46' r='36' fill='white' opacity='.22'/><path d='M82 11V19M78 15H86' stroke='{shade}' stroke-width='2' stroke-linecap='round' opacity='.5'/><g stroke-linecap='round' stroke-linejoin='round' stroke-width='3'>{ears}<path d='{head}' fill='{skin}'/><path d='M17 93Q18 76 36 75H60Q78 76 79 93' fill='{shade}'/>{chest}{eyes}{cheeks}{mouth}</g></svg>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_are_backfilled_and_preserved_on_restart() {
        let dir = crate::test_support::tempdir();
        let store = crate::store::Store::open(dir.path()).unwrap();
        store.set_user_password("alice", "test-hash").unwrap();
        let original = store.board_avatar_image("human:alice").unwrap().unwrap();
        store
            .0
            .lock()
            .unwrap()
            .execute("DELETE FROM board_default_avatars", [])
            .unwrap();
        drop(store);
        let store = crate::store::Store::open(dir.path()).unwrap();
        assert_eq!(
            store.board_avatar_image("human:alice").unwrap().unwrap(),
            original
        );
        // Existing stored portraits survive future alias changes/generator versions.
        store
            .0
            .lock()
            .unwrap()
            .execute(
                "UPDATE board_actors SET handle='renamed-alice' WHERE id='human:alice'",
                [],
            )
            .unwrap();
        drop(store);
        let store = crate::store::Store::open(dir.path()).unwrap();
        assert_eq!(
            store.board_avatar_image("human:alice").unwrap().unwrap(),
            original
        );
    }
    #[test]
    fn portraits_are_stable_varied_and_never_embed_alias_markup() {
        assert_eq!(creature("Wren"), creature("wren"));
        let pictures = (0..100)
            .map(|i| creature(&format!("person-{i}")))
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(pictures.len(), 100);
        let hostile = creature("<script>alert('alias')</script>");
        assert!(!hostile.contains("<script"));
        assert!(!hostile.contains("alert"));
        assert!(!hostile.contains("href="));
    }
}
