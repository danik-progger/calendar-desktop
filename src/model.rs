use anyhow::{Result, ensure};
use chrono::{Datelike, Duration, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::{
    backend::Api,
    style::{BIRTHDAYS_COLOR, HEALTH_COLOR, PERSONAL_COLOR, WORK_COLOR},
};

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Calendar {
    Birthdays,
    Personal,
    #[default]
    Work,
    Health,
}

impl Calendar {
    pub const ALL: [Self; 4] = [Self::Birthdays, Self::Personal, Self::Work, Self::Health];

    pub fn label(self) -> &'static str {
        match self {
            Self::Birthdays => "Birthdays",
            Self::Personal => "Personal",
            Self::Work => "Work",
            Self::Health => "Health",
        }
    }

    pub fn color(self) -> u32 {
        match self {
            Self::Birthdays => BIRTHDAYS_COLOR,
            Self::Personal => PERSONAL_COLOR,
            Self::Work => WORK_COLOR,
            Self::Health => HEALTH_COLOR,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub id: String,
    pub title: String,
    pub notes: String,
    pub calendar: Calendar,
    pub date: NaiveDate,
    pub all_day: bool,
    pub start: i32,
    pub end: i32,
    pub duration: i32,
    pub presentation: String,
}

impl Event {
    pub fn new(date: NaiveDate, minute: Option<i32>) -> Self {
        let start = minute.unwrap_or(540);
        let (start, end) = clamp_range(start, start + 45);
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            title: String::new(),
            notes: String::new(),
            calendar: Calendar::Work,
            date,
            all_day: minute.is_none(),
            start,
            end,
            duration: 45,
            presentation: "standard".into(),
        }
    }

    pub fn moved(&self, date: NaiveDate, minute: Option<i32>) -> Self {
        let mut event = self.clone();
        event.date = date;
        event.all_day = minute.is_none();
        event.duration = if self.all_day {
            self.duration
        } else {
            self.end - self.start
        };
        if let Some(start) = minute {
            (event.start, event.end) = clamp_range(start, start + event.duration);
        }
        event
    }

    pub fn resized(&self, minute: i32) -> Self {
        let mut event = self.clone();
        event.end = minute.clamp((event.start + 5).min(1440), 1440);
        event
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.id.is_empty() && !self.title.trim().is_empty(),
            "Invalid event ID or title"
        );
        ensure!(
            (1..=1440).contains(&self.duration)
                && (0..1440).contains(&self.start)
                && (self.start + 1..=1440).contains(&self.end),
            "Invalid event duration"
        );
        Ok(())
    }
}

pub fn clamp_range(start: i32, end: i32) -> (i32, i32) {
    let end = end.max(start.clamp(0, 1439) + 5).clamp(1, 1440);
    (start.clamp(0, end - 5), end)
}

pub fn snap(minute: f32) -> i32 {
    ((minute / 5.).round() as i32 * 5).clamp(0, 1440)
}

pub fn clock(minute: i32) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

pub fn event_time(minute: i32) -> String {
    if minute % 60 == 0 {
        (minute / 60).to_string()
    } else {
        format!("{}:{:02}", minute / 60, minute % 60)
    }
}

pub fn parse_time(input: &str) -> Option<i32> {
    let (hours, minutes) = input.trim().split_once(':')?;
    if hours.is_empty()
        || hours.len() > 2
        || minutes.len() != 2
        || !hours
            .bytes()
            .chain(minutes.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let hours = hours.parse::<i32>().ok()?;
    let minutes = minutes.parse::<i32>().ok()?;
    (minutes < 60 && (hours < 24 || (hours == 24 && minutes == 0))).then_some(hours * 60 + minutes)
}

pub fn monday(date: NaiveDate) -> NaiveDate {
    date - Duration::days(i64::from(date.weekday().num_days_from_monday()))
}

#[derive(Clone, Debug)]
pub struct Store {
    api: Api,
    pub events: Vec<Event>,
    pub hidden_events: usize,
    deleted: Vec<(usize, Event)>,
}

impl Store {
    pub fn remote(api: Api) -> Result<Self> {
        let loaded = api.load()?;
        Ok(Self {
            api,
            events: loaded.events,
            hidden_events: loaded.hidden_events,
            deleted: Vec::new(),
        })
    }

    pub fn save(&mut self, event: Event) -> Result<()> {
        let previous = self.events.iter().find(|old| old.id == event.id);
        if previous != Some(&event) {
            self.api.save(&event, previous.is_some())?;
            self.set_event(event);
        }
        Ok(())
    }

    fn set_event(&mut self, event: Event) {
        if let Some(existing) = self.events.iter_mut().find(|value| value.id == event.id) {
            *existing = event;
        } else {
            self.events.push(event);
        }
    }

    pub fn preview_save(&mut self, event: Event) -> Result<()> {
        event.validate()?;
        self.set_event(event);
        Ok(())
    }

    pub fn delete(&mut self, id: &str) -> Result<()> {
        if let Some(index) = self.events.iter().position(|event| event.id == id) {
            self.api.delete(id)?;
            self.deleted.push((index, self.events.remove(index)));
        }
        Ok(())
    }

    pub fn undo(&mut self) -> Result<()> {
        if let Some((index, event)) = self.deleted.last() {
            if !self.events.iter().any(|current| current.id == event.id) {
                self.api.save(event, false)?;
                self.events
                    .insert((*index).min(self.events.len()), event.clone());
            }
            self.deleted.pop();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timed_event_keeps_duration_when_dragged_through_all_day() {
        let date = NaiveDate::from_ymd_opt(2026, 11, 23).unwrap();
        let mut event = Event::new(date, Some(600));
        event.end = 720;
        let restored = event
            .moved(date.succ_opt().unwrap(), None)
            .moved(date, Some(900));
        assert_eq!((restored.start, restored.end), (900, 1020));
    }

    #[test]
    fn resize_can_end_at_midnight_when_fewer_than_five_minutes_remain() {
        let date = NaiveDate::from_ymd_opt(2026, 11, 23).unwrap();
        let event = Event {
            start: 1438,
            end: 1440,
            ..Event::new(date, Some(600))
        };
        let resized = event.resized(1435);
        assert_eq!((resized.start, resized.end), (1438, 1440));
    }
}
