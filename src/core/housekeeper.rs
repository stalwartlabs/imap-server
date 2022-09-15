/*
 * Copyright (c) 2020-2022, Stalwart Labs Ltd.
 *
 * This file is part of the Stalwart IMAP Server.
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as
 * published by the Free Software Foundation, either version 3 of
 * the License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 * in the LICENSE file at the top-level directory of this distribution.
 * You should have received a copy of the GNU Affero General Public License
 * along with this program.  If not, see <http://www.gnu.org/licenses/>.
 *
 * You can be released from the requirements of the AGPLv3 license by
 * purchasing a commercial license. Please contact licensing@stalw.art
 * for more details.
*/

use std::{sync::Arc, time::Duration};

use chrono::{Datelike, TimeZone};
use tokio::sync::watch;
use tracing::{debug, info};

use super::{
    config::{failed_to, UnwrapFailure},
    env_settings::EnvSettings,
    Core,
};

enum SimpleCron {
    EveryDay { hour: u32, minute: u32 },
    EveryWeek { day: u32, hour: u32, minute: u32 },
}

pub fn spawn_housekeeper(core: Arc<Core>, settings: &EnvSettings, mut rx: watch::Receiver<bool>) {
    let purge_cache_at = SimpleCron::parse(
        &settings
            .get("cache-purge-every")
            .unwrap_or_else(|| "0 3 *".to_string()),
    );
    let cache_ttl: u64 = settings.parse("cache-removed-id-ttl").unwrap_or(2592000);

    tokio::spawn(async move {
        debug!("Housekeeper task started.");
        loop {
            match tokio::time::timeout(purge_cache_at.time_to_next(), rx.changed()).await {
                Ok(_) => {
                    debug!("Housekeeper task exiting.");
                    return;
                }
                Err(_) => {
                    // Time to purge!
                    info!("Running housekeeper task...");
                    core.purge_deleted_ids(cache_ttl).await.ok();
                }
            }
        }
    });
}

impl SimpleCron {
    pub fn parse(value: &str) -> Self {
        let mut hour = 0;
        let mut minute = 0;

        for (pos, value) in value.split(' ').enumerate() {
            if pos == 0 {
                minute = value.parse::<u32>().failed_to("parse minute.");
                if !(0..=59).contains(&minute) {
                    failed_to(&format!("parse minute, invalid value: {}", minute));
                }
            } else if pos == 1 {
                hour = value.parse::<u32>().failed_to("parse hour.");
                if !(0..=23).contains(&hour) {
                    failed_to(&format!("parse hour, invalid value: {}", hour));
                }
            } else if pos == 2 {
                if value.as_bytes().first().failed_to("parse weekday") == &b'*' {
                    return SimpleCron::EveryDay { hour, minute };
                } else {
                    let day = value.parse::<u32>().failed_to("parse weekday.");
                    if !(1..=7).contains(&hour) {
                        failed_to(&format!(
                            "parse weekday, invalid value: {}, range is 1 (Monday) to 7 (Sunday).",
                            hour,
                        ));
                    }

                    return SimpleCron::EveryWeek { day, hour, minute };
                }
            }
        }

        failed_to("parse cron expression.");
    }

    pub fn time_to_next(&self) -> Duration {
        let now = chrono::Local::now();
        let next = match self {
            SimpleCron::EveryDay { hour, minute } => {
                let next = chrono::Local
                    .ymd(now.year(), now.month(), now.day())
                    .and_hms(*hour, *minute, 0);
                if next < now {
                    next + chrono::Duration::days(1)
                } else {
                    next
                }
            }
            SimpleCron::EveryWeek { day, hour, minute } => {
                let next = chrono::Local
                    .ymd(now.year(), now.month(), now.day())
                    .and_hms(*hour, *minute, 0);
                if next < now {
                    next + chrono::Duration::days(
                        (7 - now.weekday().number_from_monday() + *day).into(),
                    )
                } else {
                    next
                }
            }
        };

        (next - now).to_std().unwrap()
    }
}
