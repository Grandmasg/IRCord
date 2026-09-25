use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use chrono::{Datelike, Local, NaiveDate};
use sqlx::SqlitePool;
use tracing::{error, info};

pub struct BirthdayPlugin;

impl BirthdayPlugin {
    /// Berekent hoeveel dagen het nog duurt tot de eerstvolgende verjaardag
    fn days_until_birthday(day: u32, month: u32, today: NaiveDate) -> i64 {
        let current_year = today.year();

        // Probeer datum in huidig jaar (houd rekening met schrikkeljaren bijv. 29 feb)
        let this_year_date = NaiveDate::from_ymd_opt(current_year, month, day)
            .or_else(|| NaiveDate::from_ymd_opt(current_year, month, 28));

        if let Some(target) = this_year_date {
            if target >= today {
                return (target - today).num_days();
            }
        }

        // Anders volgend jaar
        let next_year_date = NaiveDate::from_ymd_opt(current_year + 1, month, day)
            .or_else(|| NaiveDate::from_ymd_opt(current_year + 1, month, 28));

        if let Some(target) = next_year_date {
            (target - today).num_days()
        } else {
            0
        }
    }

    /// Parseert een datumstring zoals "24-09", "24/09", "24-09-1995"
    fn parse_date(input: &str) -> Option<(u32, u32, Option<i32>)> {
        let clean = input.replace('/', "-").replace('.', "-");
        let parts: Vec<&str> = clean.split('-').collect();

        if parts.len() < 2 {
            return None;
        }

        let day: u32 = parts[0].trim().parse().ok()?;
        let month: u32 = parts[1].trim().parse().ok()?;

        if !(1..=31).contains(&day) || !(1..=12).contains(&month) {
            return None;
        }

        let year: Option<i32> = if parts.len() >= 3 {
            let y: i32 = parts[2].trim().parse().ok()?;
            let current_year = Local::now().year();
            if (1900..=current_year).contains(&y) {
                Some(y)
            } else {
                None
            }
        } else {
            None
        };

        Some((day, month, year))
    }

    /// Achtergrondcontrole: checkt of er vandaag leden jarig zijn die nog niet gefeliciteerd zijn
    pub async fn check_and_celebrate_birthdays(pool: &SqlitePool) -> Result<Vec<(String, String)>, Box<dyn std::error::Error + Send + Sync>> {
        let today = Local::now().date_naive();
        let current_day = today.day();
        let current_month = today.month();
        let current_year = today.year();

        let rows: Vec<(i64, String, String, String, i64, i64, Option<i64>, String)> = sqlx::query_as(
            r#"
            SELECT id, user_id, platform, display_name, day, month, year, channel
            FROM birthdays
            WHERE day = ? AND month = ? AND (last_celebrated_year IS NULL OR last_celebrated_year < ?)
            "#,
        )
        .bind(current_day as i64)
        .bind(current_month as i64)
        .bind(current_year as i64)
        .fetch_all(pool)
        .await?;

        let mut announcements = Vec::new();

        for (id, _user_id, _platform, display_name, _day, _month, year, channel) in rows {
            let message = if let Some(birth_year) = year {
                let age = current_year - (birth_year as i32);
                format!(
                    "🎂 🎉 🎈 Gefeliciteerd met je verjaardag, \x02{}\x02! Vandaag \x02{}\x02 jaar geworden! Maak er een geweldige dag van! 🥳 🍰",
                    display_name, age
                )
            } else {
                format!(
                    "🎂 🎉 🎈 Gefeliciteerd met je verjaardag, \x02{}\x02! Een hele fijne en feestelijke dag gewenst! 🥳 🍰",
                    display_name
                )
            };

            // Werk direct last_celebrated_year bij
            sqlx::query("UPDATE birthdays SET last_celebrated_year = ? WHERE id = ?")
                .bind(current_year as i64)
                .bind(id)
                .execute(pool)
                .await?;

            info!("Verjaardagsaankondiging klaargezet voor {} in kanaal {}", display_name, channel);
            announcements.push((channel, message));
        }

        Ok(announcements)
    }
}

#[async_trait]
impl Plugin for BirthdayPlugin {
    fn name(&self) -> &'static str {
        "birthday"
    }

    fn triggers(&self) -> &[&'static str] {
        &["bday", "verjaardag", "birthday"]
    }

    fn help(&self) -> &'static str {
        "!bday set <DD-MM[-JJJJ]> | !bday next | !bday [nick] | !bday del - Geautomatiseerde verjaardagsfelicitaties"
    }

    async fn on_command(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let args = cmd.args.trim();
        let today = Local::now().date_naive();

        // 1. Instellen van verjaardag: !bday set <DD-MM[-JJJJ]>
        if args.to_lowercase().starts_with("set ") {
            let date_str = args[4..].trim();
            let (day, month, year) = match Self::parse_date(date_str) {
                Some(parsed) => parsed,
                None => {
                    return Ok(Some(
                        "⚠️ Ongeldige datum. Gebruik het formaat: !bday set DD-MM (bijv. !bday set 24-09) of met jaar (bijv. !bday set 24-09-1995)".to_string(),
                    ));
                }
            };

            let days_left = Self::days_until_birthday(day, month, today);
            let year_val = year.map(|y| y as i64);

            sqlx::query(
                r#"
                INSERT INTO birthdays (user_id, platform, display_name, day, month, year, channel)
                VALUES (?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(user_id, platform) DO UPDATE SET
                    display_name = excluded.display_name,
                    day = excluded.day,
                    month = excluded.month,
                    year = excluded.year,
                    channel = excluded.channel
                "#,
            )
            .bind(&cmd.author)
            .bind(&cmd.platform)
            .bind(&cmd.author)
            .bind(day as i64)
            .bind(month as i64)
            .bind(year_val)
            .bind(&cmd.channel)
            .execute(&ctx.db)
            .await?;

            let year_str = match year {
                Some(y) => format!("-{}", y),
                None => String::new(),
            };

            let days_str = if days_left == 0 {
                "VANDAAG! Gefeliciteerd! 🎉".to_string()
            } else if days_left == 1 {
                "morgen! 🎈".to_string()
            } else {
                format!("over {} dagen 🎈", days_left)
            };

            return Ok(Some(format!(
                "🎂 [Verjaardag] Opgeslagen voor \x02{}\x02! Jouw verjaardag staat op \x02{:02}-{:02}{}\x02 (dat is {}). Ik zal je 's ochtends feliciteren in {}!",
                cmd.author, day, month, year_str, days_str, cmd.channel
            )));
        }

        // 2. Verwijderen: !bday del / !bday remove
        if args.eq_ignore_ascii_case("del") || args.eq_ignore_ascii_case("remove") {
            let res = sqlx::query("DELETE FROM birthdays WHERE user_id = ? AND platform = ?")
                .bind(&cmd.author)
                .bind(&cmd.platform)
                .execute(&ctx.db)
                .await?;

            if res.rows_affected() > 0 {
                return Ok(Some(format!(
                    "🗑️ [Verjaardag] Jouw verjaardag is succesvol verwijderd voor \x02{}\x02.",
                    cmd.author
                )));
            } else {
                return Ok(Some("ℹ️ Je had nog geen verjaardag geregistreerd.".to_string()));
            }
        }

        // 3. Eerstvolgende verjaardagen bekijken: !bday next / !bday list / !bday upcoming
        if args.eq_ignore_ascii_case("next")
            || args.eq_ignore_ascii_case("upcoming")
            || args.eq_ignore_ascii_case("list")
        {
            let rows: Vec<(String, i64, i64, Option<i64>)> = sqlx::query_as(
                "SELECT display_name, day, month, year FROM birthdays ORDER BY id ASC",
            )
            .fetch_all(&ctx.db)
            .await?;

            if rows.is_empty() {
                return Ok(Some("ℹ️ Er zijn nog geen verjaardagen geregistreerd. Stel de jouwe in met !bday set DD-MM!".to_string()));
            }

            let mut list: Vec<(String, u32, u32, Option<i32>, i64)> = rows
                .into_iter()
                .map(|(name, d, m, y)| {
                    let day = d as u32;
                    let month = m as u32;
                    let year = y.map(|val| val as i32);
                    let days = Self::days_until_birthday(day, month, today);
                    (name, day, month, year, days)
                })
                .collect();

            // Sorteer op aantal resterende dagen oplopend
            list.sort_by_key(|item| item.4);

            let top_items: Vec<String> = list
                .into_iter()
                .take(5)
                .map(|(name, day, month, _year, days)| {
                    if days == 0 {
                        format!("\x02{}\x02 ({:02}-{:02}, VANDAAG! 🎉)", name, day, month)
                    } else if days == 1 {
                        format!("\x02{}\x02 ({:02}-{:02}, morgen)", name, day, month)
                    } else {
                        format!("\x02{}\x02 ({:02}-{:02}, over {}d)", name, day, month, days)
                    }
                })
                .collect();

            return Ok(Some(format!(
                "🎂 [Eerstvolgende Verjaardagen] {}",
                top_items.join(" | ")
            )));
        }

        // 4. Verjaardag van specifieke nick of van jezelf opvragen
        let target = if args.is_empty() {
            cmd.author.as_str()
        } else {
            args
        };

        let row: Option<(String, i64, i64, Option<i64>)> = sqlx::query_as(
            r#"
            SELECT display_name, day, month, year
            FROM birthdays
            WHERE LOWER(display_name) = LOWER(?) OR LOWER(user_id) = LOWER(?)
            LIMIT 1
            "#,
        )
        .bind(target)
        .bind(target)
        .fetch_optional(&ctx.db)
        .await?;

        if let Some((name, d, m, y)) = row {
            let day = d as u32;
            let month = m as u32;
            let days_left = Self::days_until_birthday(day, month, today);

            let year_info = if let Some(birth_year) = y {
                let age = today.year() - (birth_year as i32);
                let age_str = if days_left == 0 {
                    format!(" (vandaag {} jaar geworden!)", age)
                } else {
                    format!(" (wordt {} jaar)", age)
                };
                age_str
            } else {
                String::new()
            };

            let days_str = if days_left == 0 {
                "is \x02VANDAAG\x02 jarig! 🎉🎂".to_string()
            } else if days_left == 1 {
                "is \x02morgen\x02 jarig! 🎈".to_string()
            } else {
                format!("is jarig over \x02{}\x02 dagen", days_left)
            };

            Ok(Some(format!(
                "🎂 [Verjaardag] \x02{}\x02 is jarig op \x02{:02}-{:02}\x02{} en {}.",
                name, day, month, year_info, days_str
            )))
        } else if args.is_empty() {
            Ok(Some(
                "ℹ️ Je hebt nog geen verjaardag ingesteld. Gebruik: !bday set DD-MM (of DD-MM-JJJJ) | !bday next".to_string(),
            ))
        } else {
            Ok(Some(format!(
                "ℹ️ Geen verjaardag gevonden voor '{}'. Stel in met: !bday set DD-MM",
                target
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn test_parse_date() {
        assert_eq!(BirthdayPlugin::parse_date("24-09"), Some((24, 9, None)));
        assert_eq!(BirthdayPlugin::parse_date("24/09"), Some((24, 9, None)));
        assert_eq!(BirthdayPlugin::parse_date("24.09"), Some((24, 9, None)));
        assert_eq!(BirthdayPlugin::parse_date("24-09-1995"), Some((24, 9, Some(1995))));
        assert_eq!(BirthdayPlugin::parse_date("32-09"), None);
        assert_eq!(BirthdayPlugin::parse_date("10-13"), None);
        assert_eq!(BirthdayPlugin::parse_date("ongeldig"), None);
    }

    #[test]
    fn test_days_until_birthday() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 24).unwrap();
        // Zelfde dag: 0 dagen
        assert_eq!(BirthdayPlugin::days_until_birthday(24, 9, today), 0);
        // Volgende dag: 1 dag
        assert_eq!(BirthdayPlugin::days_until_birthday(25, 9, today), 1);
        // Gisteren: volgend jaar (364 dagen)
        assert_eq!(BirthdayPlugin::days_until_birthday(23, 9, today), 364);
    }
}
