//! KAS/USD-Preis aus mehreren öffentlichen Quellen, deterministischer Median.
//! Endpunkte am 28.09.2026 geprüft (alle lieferten ≈ 0,0467 USD).
//! USDT-Paare werden als USD gewertet (bewusste Vereinfachung, im Orakel-
//! Komitee später durch einen USDT/USD-Kurs zu ersetzen).

use serde_json::Value;
use std::time::Duration;

pub struct Quote {
    pub source: &'static str,
    pub usd: f64,
}

type Parse = fn(&Value) -> Option<f64>;

fn num(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

const SOURCES: [(&str, &str, Parse); 6] = [
    ("api.kaspa.org", "https://api.kaspa.org/info/price", |v| num(&v["price"])),
    ("CoinGecko", "https://api.coingecko.com/api/v3/simple/price?ids=kaspa&vs_currencies=usd", |v| num(&v["kaspa"]["usd"])),
    ("MEXC", "https://api.mexc.com/api/v3/ticker/price?symbol=KASUSDT", |v| num(&v["price"])),
    ("Gate", "https://api.gateio.ws/api/v4/spot/tickers?currency_pair=KAS_USDT", |v| num(&v[0]["last"])),
    ("KuCoin", "https://api.kucoin.com/api/v1/market/orderbook/level1?symbol=KAS-USDT", |v| num(&v["data"]["price"])),
    ("Bybit", "https://api.bybit.com/v5/market/tickers?category=spot&symbol=KASUSDT", |v| num(&v["result"]["list"][0]["lastPrice"])),
];

pub fn fetch_all() -> Vec<Result<Quote, String>> {
    // IPv4 zuerst: Ist IPv6 kaputt (28.09.2026 gemessen), verbrauchte der
    // erste IPv6-Versuch das ganze Zeitlimit, und die Hälfte der Quellen fiel aus.
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(8))
        .resolver(|addr: &str| -> std::io::Result<Vec<std::net::SocketAddr>> {
            use std::net::ToSocketAddrs;
            let mut v: Vec<_> = addr.to_socket_addrs()?.collect();
            v.sort_by_key(|a| a.is_ipv6());
            Ok(v)
        })
        .build();
    SOURCES
        .iter()
        .map(|(name, url, parse)| {
            let v: Value = agent.get(url).call().map_err(|e| format!("{name}: {e}"))?.into_json().map_err(|e| format!("{name}: {e}"))?;
            let usd = parse(&v).ok_or(format!("{name}: Antwort ohne Preis"))?;
            if usd.is_finite() && usd > 0.0 { Ok(Quote { source: name, usd }) } else { Err(format!("{name}: ungültiger Preis {usd}")) }
        })
        .collect()
}

/// Median aus mindestens 3 Quellen; Quellen, die mehr als `max_dev` (z. B.
/// 0,03 = 3 %) vom Median abweichen, fallen heraus, danach neuer Median.
/// Ergebnis in USD je KAS × 1e8 (Einheit von risk_oracle.sil).
pub fn median_price(quotes: &[Quote], max_dev: f64) -> Result<i64, String> {
    fn median(mut xs: Vec<f64>) -> f64 {
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = xs.len();
        if n % 2 == 1 { xs[n / 2] } else { (xs[n / 2 - 1] + xs[n / 2]) / 2.0 }
    }
    if quotes.len() < 3 {
        return Err(format!("nur {} Quellen erreichbar, mindestens 3 nötig", quotes.len()));
    }
    let m = median(quotes.iter().map(|q| q.usd).collect());
    let kept: Vec<f64> = quotes.iter().map(|q| q.usd).filter(|p| ((p - m) / m).abs() <= max_dev).collect();
    if kept.len() < 3 {
        return Err(format!("Quellen widersprechen sich (nur {} innerhalb {:.0} %)", kept.len(), max_dev * 100.0));
    }
    Ok((median(kept) * 1e8).round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn q(v: &[f64]) -> Vec<Quote> {
        v.iter().map(|&usd| Quote { source: "t", usd }).collect()
    }
    #[test]
    fn median_und_ausreisser() {
        assert_eq!(median_price(&q(&[0.046, 0.047, 0.048]), 0.03).unwrap(), 4_700_000);
        // ein Ausreißer fliegt raus, drei bleiben
        assert_eq!(median_price(&q(&[0.0465, 0.047, 0.0475, 0.2]), 0.03).unwrap(), 4_700_000);
        // 0.046 läge 3,2 % unter dem Median 0.0475 und fiele heraus → nur 2 übrig
        assert!(median_price(&q(&[0.046, 0.047, 0.048, 0.2]), 0.03).is_err());
        assert!(median_price(&q(&[0.046, 0.047]), 0.03).is_err());
        assert!(median_price(&q(&[0.01, 0.05, 0.2]), 0.03).is_err());
    }
}
