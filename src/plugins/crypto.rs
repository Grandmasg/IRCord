use super::{CommandEvent, Plugin, PluginContext};
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;

pub struct CryptoPlugin;

#[derive(Deserialize)]
struct CoinPriceData {
    eur: Option<f64>,
    usd: Option<f64>,
    eur_24h_change: Option<f64>,
    usd_24h_change: Option<f64>,
}

#[derive(Deserialize)]
struct FrankfurterResponse {
    amount: f64,
    base: String,
    rates: HashMap<String, f64>,
}

#[async_trait]
impl Plugin for CryptoPlugin {
    fn name(&self) -> &'static str {
        "crypto"
    }

    fn triggers(&self) -> &[&'static str] {
        &["crypto", "coin", "currency", "valuta", "fx", "koers"]
    }

    fn help(&self) -> &'static str {
        "!crypto <coin> / !currency <amount> <from> <to> | !valuta <bedrag> <van> <naar>"
    }

    async fn on_command(
        &self,
        ctx: &PluginContext,
        cmd: &CommandEvent,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let trigger = cmd.trigger.as_str();

        if trigger == "valuta" || trigger == "fx" || trigger == "currency" {
            return self.handle_valuta(ctx, &cmd.args).await;
        }

        self.handle_crypto(ctx, &cmd.args).await
    }
}

impl CryptoPlugin {
    async fn handle_crypto(
        &self,
        ctx: &PluginContext,
        args: &str,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let raw = args.trim().to_lowercase();
        let ticker = if raw.is_empty() { "btc" } else { raw.as_str() };

        let (coin_id, display_name, display_symbol) = match ticker {
            "btc" | "bitcoin" => ("bitcoin", "Bitcoin", "BTC"),
            "eth" | "ethereum" => ("ethereum", "Ethereum", "ETH"),
            "sol" | "solana" => ("solana", "Solana", "SOL"),
            "xrp" | "ripple" => ("ripple", "XRP", "XRP"),
            "doge" | "dogecoin" => ("dogecoin", "Dogecoin", "DOGE"),
            "ada" | "cardano" => ("cardano", "Cardano", "ADA"),
            "bnb" | "binancecoin" => ("binancecoin", "BNB", "BNB"),
            "dot" | "polkadot" => ("polkadot", "Polkadot", "DOT"),
            "avax" | "avalanche" => ("avalanche-2", "Avalanche", "AVAX"),
            "link" | "chainlink" => ("chainlink", "Chainlink", "LINK"),
            "ltc" | "litecoin" => ("litecoin", "Litecoin", "LTC"),
            "monero" | "xmr" => ("monero", "Monero", "XMR"),
            other => (other, other, other),
        };

        let url = format!(
            "https://api.coingecko.com/api/v3/simple/price?ids={}&vs_currencies=eur,usd&include_24hr_change=true",
            coin_id
        );

        let mut req = ctx
            .http
            .get(&url)
            .header("User-Agent", "IRCordBot/1.0 (crypto client)");

        if let Ok(key) = std::env::var("COINGECKO_API_KEY") {
            let k = key.trim();
            if !k.is_empty() {
                req = req.header("x-cg-demo-api-key", k);
            }
        }

        let resp = req.send().await?;

        if !resp.status().is_success() {
            return Ok(Some(format!(
                "🪙 {}",
                ctx.locale.tf("crypto_error", &[("ticker", ticker)])
            )));
        }

        let map: HashMap<String, CoinPriceData> = resp.json().await?;
        let data = match map.get(coin_id) {
            Some(d) => d,
            None => {
                return Ok(Some(format!(
                    "🪙 {}",
                    ctx.locale.tf("crypto_not_found", &[("ticker", ticker)])
                )))
            }
        };

        let eur = data.eur.unwrap_or(0.0);
        let usd = data.usd.unwrap_or(0.0);
        let change_24h = data.eur_24h_change.or(data.usd_24h_change).unwrap_or(0.0);

        let trend_icon = if change_24h >= 0.0 { "📈 +" } else { "📉 " };
        let period_label = if ctx.locale.is_dutch() { "24u" } else { "24h" };
        let title = ctx.locale.t("currency_crypto_title");

        Ok(Some(format!(
            "🪙 [{}] {} ({}): €{} / ${} | {}: {}{:.2}%",
            title,
            display_name,
            display_symbol.to_uppercase(),
            format_price(eur),
            format_price(usd),
            period_label,
            trend_icon,
            change_24h
        )))
    }

    async fn handle_valuta(
        &self,
        ctx: &PluginContext,
        args: &str,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        let parts: Vec<&str> = args.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(Some(ctx.locale.t("currency_usage").into()));
        }

        let (amount, from_curr, to_curr) = if parts.len() == 1 {
            (1.0, parts[0].to_uppercase(), "EUR".to_string())
        } else if parts.len() == 2 {
            if let Ok(num) = parts[0].parse::<f64>() {
                (num, parts[1].to_uppercase(), "EUR".to_string())
            } else {
                (1.0, parts[0].to_uppercase(), parts[1].to_uppercase())
            }
        } else {
            let num = parts[0].parse::<f64>().unwrap_or(1.0);
            let from = parts[1].to_uppercase();
            // Support 'naar', 'to', 'nach' or 'in': !valuta 100 usd to eur
            let to = if parts.len() >= 4 && (parts[2].eq_ignore_ascii_case("naar") || parts[2].eq_ignore_ascii_case("to") || parts[2].eq_ignore_ascii_case("nach") || parts[2].eq_ignore_ascii_case("in")) {
                parts[3].to_uppercase()
            } else {
                parts[2].to_uppercase()
            };
            (num, from, to)
        };

        let url = format!(
            "https://api.frankfurter.app/latest?amount={}&from={}&to={}",
            amount, from_curr, to_curr
        );

        let resp = ctx.http.get(&url).send().await?;
        if !resp.status().is_success() {
            return Ok(Some(format!(
                "💱 {}",
                ctx.locale.tf("currency_not_found", &[("from", &from_curr), ("to", &to_curr)])
            )));
        }

        let data: FrankfurterResponse = resp.json().await?;
        let rate_val = data.rates.get(&to_curr).copied().unwrap_or(0.0);
        let single_rate = if data.amount > 0.0 { rate_val / data.amount } else { rate_val };
        let fx_title = ctx.locale.t("currency_fx_title");
        let rate_title = ctx.locale.t("currency_rate");

        Ok(Some(format!(
            "💱 [{}] {:.2} {} = {:.2} {} ({}: 1 {} = {:.4} {})",
            fx_title, data.amount, data.base, rate_val, to_curr, rate_title, data.base, single_rate, to_curr
        )))
    }
}

fn format_price(p: f64) -> String {
    if p >= 1.0 {
        format!("{:.2}", p)
    } else if p >= 0.0001 {
        format!("{:.4}", p)
    } else {
        format!("{:.8}", p)
    }
}
