//! UTC minute slots; no catch-up queue and no duplicate slot after clock rollback.
use anyhow::{Result, anyhow, bail};
#[derive(Clone, Debug)]
pub struct Cron {
    pub expression: String,
    fields: Vec<Vec<bool>>,
    dom_any: bool,
    dow_any: bool,
}
impl Cron {
    pub fn parse(text: &str) -> Result<Self> {
        let parts = text.split_whitespace().collect::<Vec<_>>();
        if parts.len() != 5 {
            bail!("cron requires five UTC fields");
        }
        let ranges = [(0, 59), (0, 23), (1, 31), (1, 12), (0, 7)];
        let mut fields = vec![];
        for (field, (min, max)) in parts.iter().zip(ranges) {
            let mut values = vec![false; max + 1];
            for term in field.split(',') {
                let mut split = term.split('/');
                let range = split.next().unwrap();
                let step = split
                    .next()
                    .map(str::parse::<usize>)
                    .transpose()?
                    .unwrap_or(1);
                if step == 0 || split.next().is_some() {
                    bail!("invalid cron step");
                }
                let (start, end) = if range == "*" {
                    (min, max)
                } else if let Some((a, b)) = range.split_once('-') {
                    (a.parse()?, b.parse()?)
                } else {
                    let start = range.parse()?;
                    (start, if term.contains('/') { max } else { start })
                };
                if start < min || end > max || start > end {
                    bail!("cron field out of range");
                }
                for i in (start..=end).step_by(step) {
                    values[i] = true;
                }
            }
            fields.push(values);
        }
        fields[4][0] |= fields[4][7];
        Ok(Self {
            expression: parts.join(" "),
            fields,
            dom_any: parts[2] == "*",
            dow_any: parts[4] == "*",
        })
    }
    pub fn matches(&self, epoch_ms: i64) -> bool {
        let sec = epoch_ms.div_euclid(1000) as libc::time_t;
        let mut tm = std::mem::MaybeUninit::<libc::tm>::uninit();
        let tm = unsafe {
            if libc::gmtime_r(&sec, tm.as_mut_ptr()).is_null() {
                return false;
            }
            tm.assume_init()
        };
        let dom = self.fields[2][tm.tm_mday as usize];
        let dow = self.fields[4][tm.tm_wday as usize];
        self.fields[0][tm.tm_min as usize]
            && self.fields[1][tm.tm_hour as usize]
            && self.fields[3][tm.tm_mon as usize + 1]
            && if self.dom_any || self.dow_any {
                dom && dow
            } else {
                dom || dow
            }
    }
    pub fn next(&self, after_ms: i64) -> Result<i64> {
        let start = after_ms
            .div_euclid(60_000)
            .checked_add(1)
            .and_then(|m| m.checked_mul(60_000))
            .ok_or_else(|| anyhow!("cron time overflow"))?;
        for minute in 0..(8 * 366 * 24 * 60i64) {
            let at = start
                .checked_add(minute * 60_000)
                .ok_or_else(|| anyhow!("cron time overflow"))?;
            if self.matches(at) {
                return Ok(at);
            }
        }
        bail!("cron has no occurrence within eight years")
    }
}
#[derive(Default)]
pub struct Schedule {
    high_water: Option<i64>,
}
impl Schedule {
    pub fn advance(&mut self, now_ms: i64) -> Option<i64> {
        let minute = now_ms.div_euclid(60_000);
        if self.high_water.is_none_or(|old| minute > old) {
            let prior = self.high_water.replace(minute);
            prior.map(|_| minute * 60_000)
        } else {
            None
        }
    }
}
