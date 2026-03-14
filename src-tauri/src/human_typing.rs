use rand::Rng;
use std::collections::{HashMap, HashSet};

const PROB_ERROR: f64 = 0.04;
const PROB_SWAP_ERROR: f64 = 0.015;
const PROB_NOTICE_ERROR: f64 = 0.85;
const SPEED_BOOST_COMMON_WORD: f64 = 0.6;
const SPEED_PENALTY_COMPLEX_WORD: f64 = 1.3;
const SPEED_BOOST_CLOSE_KEYS: f64 = 0.5;
const SPEED_BOOST_BIGRAM: f64 = 0.4;
const TIME_KEYSTROKE_STD: f64 = 0.03;
const TIME_BACKSPACE_MEAN: f64 = 0.12;
const TIME_BACKSPACE_STD: f64 = 0.02;
const TIME_REACTION_MEAN: f64 = 0.35;
const TIME_REACTION_STD: f64 = 0.1;
const TIME_UPPERCASE_PENALTY: f64 = 0.2;
const TIME_SPACE_PAUSE_MEAN: f64 = 0.25;
const TIME_SPACE_PAUSE_STD: f64 = 0.05;
const FATIGUE_FACTOR: f64 = 1.0005;
const AVG_WORD_LENGTH: f64 = 5.0;
const WPM_STD: f64 = 10.0;
const DEFAULT_WPM: f64 = 80.0;

#[derive(Debug, Clone)]
pub enum TypingAction {
  Char(char),
  Backspace,
}

#[derive(Debug, Clone)]
pub struct TypingEvent {
  pub time: f64,
  pub action: TypingAction,
}

struct KeyboardLayout {
  pos_map: HashMap<char, (usize, usize)>,
  grid: Vec<Vec<char>>,
}

impl KeyboardLayout {
  fn new() -> Self {
    let grid: Vec<Vec<char>> = vec![
      "`1234567890-=".chars().collect(),
      "qwertyuiop[]\\".chars().collect(),
      "asdfghjkl;'".chars().collect(),
      "zxcvbnm,./".chars().collect(),
    ];
    let mut pos_map = HashMap::new();
    for (r, row) in grid.iter().enumerate() {
      for (c, &ch) in row.iter().enumerate() {
        pos_map.insert(ch, (r, c));
      }
    }
    KeyboardLayout { pos_map, grid }
  }

  fn has_key(&self, ch: char) -> bool {
    self.pos_map.contains_key(&ch.to_ascii_lowercase())
  }

  fn get_neighbor_keys(&self, ch: char) -> Vec<char> {
    let ch = ch.to_ascii_lowercase();
    let (r, c) = match self.pos_map.get(&ch) {
      Some(&pos) => pos,
      None => return vec![],
    };
    let deltas: [(i32, i32); 8] = [
      (-1, -1),
      (-1, 0),
      (-1, 1),
      (0, -1),
      (0, 1),
      (1, -1),
      (1, 0),
      (1, 1),
    ];
    let mut neighbors = Vec::new();
    for (dr, dc) in &deltas {
      let nr = r as i32 + dr;
      let nc = c as i32 + dc;
      if nr >= 0 && (nr as usize) < self.grid.len() {
        let row = &self.grid[nr as usize];
        if nc >= 0 && (nc as usize) < row.len() {
          neighbors.push(row[nc as usize]);
        }
      }
    }
    neighbors
  }

  fn get_distance(&self, c1: char, c2: char) -> f64 {
    let c1 = c1.to_ascii_lowercase();
    let c2 = c2.to_ascii_lowercase();
    match (self.pos_map.get(&c1), self.pos_map.get(&c2)) {
      (Some(&(r1, c1p)), Some(&(r2, c2p))) => {
        let dr = r1 as f64 - r2 as f64;
        let dc = c1p as f64 - c2p as f64;
        (dr * dr + dc * dc).sqrt()
      }
      _ => 4.0,
    }
  }

  fn get_random_neighbor(&self, ch: char, rng: &mut impl Rng) -> char {
    let neighbors = self.get_neighbor_keys(ch);
    if neighbors.is_empty() {
      let flat: Vec<char> = self.grid.iter().flat_map(|r| r.iter().copied()).collect();
      flat[rng.random_range(0..flat.len())]
    } else {
      neighbors[rng.random_range(0..neighbors.len())]
    }
  }
}

fn normal_sample(rng: &mut impl Rng, mean: f64, std_dev: f64) -> f64 {
  // Box-Muller transform
  let u1: f64 = rng.random::<f64>().max(1e-10);
  let u2: f64 = rng.random::<f64>();
  let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
  mean + std_dev * z
}

static COMMON_WORDS: &[&str] = &[
  "the", "be", "to", "of", "and", "a", "in", "that", "have", "it", "for", "not", "on", "with",
  "he", "as", "you", "do", "at", "this", "but", "his", "by", "from", "they", "we", "say", "her",
  "she", "or", "an", "will", "my", "one", "all", "would", "there", "their", "what", "so", "up",
  "out", "if", "about", "who", "get", "which", "go", "me", "when", "make", "can", "like", "time",
  "no", "just", "him", "know", "take", "people", "into", "year", "your", "good", "some", "could",
  "them", "see", "other", "than", "then", "now", "look", "only", "come", "its", "over", "think",
  "also", "back", "after", "use", "two", "how", "our", "work", "first", "well", "way", "even",
  "new", "want", "because",
];

static COMMON_BIGRAMS: &[&str] = &[
  "th", "he", "in", "er", "an", "re", "on", "at", "en", "nd", "ti", "es", "or", "te", "of", "ed",
  "is", "it", "al", "ar", "st", "to", "nt", "ng", "se", "ha", "as", "ou", "io", "le", "ve", "co",
  "me", "de", "hi", "ri", "ro", "ic", "ne", "ea", "ra", "ce",
];

fn get_word_difficulty(word: &str) -> &'static str {
  let lower = word.to_lowercase();
  let trimmed = lower.trim_matches(|c: char| matches!(c, '.' | ',' | '!' | '?' | ';' | ':'));
  let common_set: HashSet<&str> = COMMON_WORDS.iter().copied().collect();
  if common_set.contains(trimmed) {
    return "common";
  }
  let is_long = trimmed.len() > 8;
  let has_complex = trimmed.chars().any(|c| matches!(c, 'z' | 'x' | 'q' | 'j'));
  if is_long || has_complex {
    return "complex";
  }
  "normal"
}

fn is_common_bigram(c1: char, c2: char) -> bool {
  let bigram = format!("{}{}", c1.to_ascii_lowercase(), c2.to_ascii_lowercase());
  let bigram_set: HashSet<&str> = COMMON_BIGRAMS.iter().copied().collect();
  bigram_set.contains(bigram.as_str())
}

pub struct MarkovTyper {
  target: Vec<char>,
  current: Vec<char>,
  keyboard: KeyboardLayout,
  base_keystroke_time: f64,
  fatigue_multiplier: f64,
  mental_cursor_pos: usize,
  last_char_typed: Option<char>,
  total_time: f64,
  last_was_backspace: bool,
  rng: rand::rngs::ThreadRng,
}

impl MarkovTyper {
  pub fn new(text: &str, wpm: Option<f64>) -> Self {
    let mut rng = rand::rng();
    let target_wpm = wpm.unwrap_or(DEFAULT_WPM);
    let session_wpm = normal_sample(&mut rng, target_wpm, WPM_STD).max(10.0);
    let base_keystroke_time = 60.0 / (session_wpm * AVG_WORD_LENGTH);

    MarkovTyper {
      target: text.chars().collect(),
      current: Vec::new(),
      keyboard: KeyboardLayout::new(),
      base_keystroke_time,
      fatigue_multiplier: 1.0,
      mental_cursor_pos: 0,
      last_char_typed: None,
      total_time: 0.0,
      last_was_backspace: false,
      rng,
    }
  }

  fn get_current_word(&self) -> Option<String> {
    if self.mental_cursor_pos >= self.target.len() {
      return None;
    }
    let mut start = self.mental_cursor_pos;
    while start > 0 && self.target[start - 1] != ' ' {
      start -= 1;
    }
    let mut end = self.mental_cursor_pos;
    while end < self.target.len() && self.target[end] != ' ' {
      end += 1;
    }
    Some(self.target[start..end].iter().collect())
  }

  fn calculate_keystroke_time(&mut self, ch: char) -> f64 {
    let mut time = self.base_keystroke_time * self.fatigue_multiplier;

    if let Some(word) = self.get_current_word() {
      match get_word_difficulty(&word) {
        "common" => time *= SPEED_BOOST_COMMON_WORD,
        "complex" => time *= SPEED_PENALTY_COMPLEX_WORD,
        _ => {}
      }
    }

    if let Some(last) = self.last_char_typed {
      if is_common_bigram(last, ch) {
        time *= SPEED_BOOST_BIGRAM;
      } else {
        let dist = self.keyboard.get_distance(last, ch);
        if dist > 0.0 && dist < 2.0 {
          time *= SPEED_BOOST_CLOSE_KEYS;
        } else if dist > 4.0 {
          time *= 1.2;
        }
      }
    }

    if ch == ' ' {
      time += normal_sample(&mut self.rng, TIME_SPACE_PAUSE_MEAN, TIME_SPACE_PAUSE_STD);
    } else if ch.is_uppercase() {
      time += TIME_UPPERCASE_PENALTY;
    }

    let dt = normal_sample(&mut self.rng, time, TIME_KEYSTROKE_STD);
    dt.max(0.02)
  }

  fn step(&mut self) -> Option<TypingEvent> {
    if self.current == self.target {
      return None;
    }

    // Find first error position
    let mut first_error_pos = self.target.len();
    let min_len = self.current.len().min(self.target.len());
    for i in 0..min_len {
      if self.current[i] != self.target[i] {
        first_error_pos = i;
        break;
      }
    }
    if self.current.len() > self.target.len() && first_error_pos == self.target.len() {
      first_error_pos = self.target.len();
    }

    // Error correction
    if first_error_pos < self.current.len() {
      let mut should_correct = false;

      if self.last_was_backspace || self.mental_cursor_pos >= self.target.len() {
        should_correct = true;
      } else if !self.current.is_empty() {
        let last_char = *self.current.last().unwrap();
        let distance = self.current.len() - first_error_pos;

        if " \n\t.,;!?:()[]{}\"'<>".contains(last_char) {
          should_correct = true;
        } else if distance >= 2 {
          if self.rng.random::<f64>() < 0.8 {
            should_correct = true;
          }
        } else if distance == 1 && self.rng.random::<f64>() < PROB_NOTICE_ERROR {
          should_correct = true;
        }
      }

      if should_correct {
        if !self.last_was_backspace {
          let dt = normal_sample(&mut self.rng, TIME_REACTION_MEAN, TIME_REACTION_STD).max(0.1);
          self.total_time += dt;
        }

        let dt = normal_sample(&mut self.rng, TIME_BACKSPACE_MEAN, TIME_BACKSPACE_STD);
        self.total_time += dt;
        self.current.pop();
        self.mental_cursor_pos = self.current.len();
        self.last_was_backspace = true;

        return Some(TypingEvent {
          time: self.total_time,
          action: TypingAction::Backspace,
        });
      }
    }

    self.last_was_backspace = false;

    if self.mental_cursor_pos > self.current.len() {
      self.mental_cursor_pos = self.current.len();
    }
    if self.mental_cursor_pos >= self.target.len() {
      return None;
    }

    let char_intended = self.target[self.mental_cursor_pos];
    self.fatigue_multiplier *= FATIGUE_FACTOR;

    // Non-QWERTY characters (CJK, Cyrillic, etc.) are composed via IME —
    // skip error simulation entirely, just apply realistic timing.
    let on_keyboard = self.keyboard.has_key(char_intended);

    // Swap error (only for characters on the physical keyboard)
    if on_keyboard && self.mental_cursor_pos + 1 < self.target.len() {
      let char_after = self.target[self.mental_cursor_pos + 1];
      if char_after != ' '
        && char_after != char_intended
        && self.keyboard.has_key(char_after)
        && self.rng.random::<f64>() < PROB_SWAP_ERROR
      {
        let dt = self.calculate_keystroke_time(char_after);
        self.total_time += dt;
        self.current.push(char_after);
        self.last_char_typed = Some(char_after);
        self.mental_cursor_pos += 1;
        return Some(TypingEvent {
          time: self.total_time,
          action: TypingAction::Char(char_after),
        });
      }
    }

    // Normal typing with possible error (errors only for QWERTY characters)
    let typed_char = if on_keyboard {
      let mut current_prob_error = PROB_ERROR;
      if let Some(word) = self.get_current_word() {
        match get_word_difficulty(&word) {
          "complex" => current_prob_error *= 1.5,
          "common" => current_prob_error *= 0.5,
          _ => {}
        }
      }
      if self.rng.random::<f64>() < current_prob_error {
        self
          .keyboard
          .get_random_neighbor(char_intended, &mut self.rng)
      } else {
        char_intended
      }
    } else {
      char_intended
    };

    let dt = self.calculate_keystroke_time(typed_char);
    self.total_time += dt;
    self.current.push(typed_char);
    self.last_char_typed = Some(typed_char);
    self.mental_cursor_pos += 1;

    Some(TypingEvent {
      time: self.total_time,
      action: TypingAction::Char(typed_char),
    })
  }

  pub fn run(mut self) -> Vec<TypingEvent> {
    let max_steps = self.target.len() * 10;
    let mut events = Vec::new();
    let mut steps = 0;
    while let Some(event) = self.step() {
      events.push(event);
      steps += 1;
      if steps > max_steps {
        break;
      }
    }
    events
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_generates_events() {
    let typer = MarkovTyper::new("hello", Some(60.0));
    let events = typer.run();
    assert!(!events.is_empty());
    // Final text should be "hello" — verify by replaying
    let mut text = String::new();
    for event in &events {
      match &event.action {
        TypingAction::Char(c) => text.push(*c),
        TypingAction::Backspace => {
          text.pop();
        }
      }
    }
    assert_eq!(text, "hello");
  }

  #[test]
  fn test_timing_increases() {
    let typer = MarkovTyper::new("test", Some(60.0));
    let events = typer.run();
    for window in events.windows(2) {
      assert!(window[1].time >= window[0].time);
    }
  }

  #[test]
  fn test_empty_text() {
    let typer = MarkovTyper::new("", Some(60.0));
    let events = typer.run();
    assert!(events.is_empty());
  }

  #[test]
  fn test_chinese_text() {
    let input = "你好世界";
    let typer = MarkovTyper::new(input, Some(60.0));
    let events = typer.run();
    let mut text = String::new();
    for event in &events {
      match &event.action {
        TypingAction::Char(c) => text.push(*c),
        TypingAction::Backspace => {
          text.pop();
        }
      }
    }
    assert_eq!(text, input);
  }

  #[test]
  fn test_russian_text() {
    let input = "Привет мир";
    let typer = MarkovTyper::new(input, Some(60.0));
    let events = typer.run();
    let mut text = String::new();
    for event in &events {
      match &event.action {
        TypingAction::Char(c) => text.push(*c),
        TypingAction::Backspace => {
          text.pop();
        }
      }
    }
    assert_eq!(text, input);
  }

  #[test]
  fn test_japanese_text() {
    let input = "東京タワー";
    let typer = MarkovTyper::new(input, Some(60.0));
    let events = typer.run();
    let mut text = String::new();
    for event in &events {
      match &event.action {
        TypingAction::Char(c) => text.push(*c),
        TypingAction::Backspace => {
          text.pop();
        }
      }
    }
    assert_eq!(text, input);
  }

  #[test]
  fn test_mixed_latin_and_cjk() {
    let input = "Hello 你好 world";
    let typer = MarkovTyper::new(input, Some(60.0));
    let events = typer.run();
    let mut text = String::new();
    for event in &events {
      match &event.action {
        TypingAction::Char(c) => text.push(*c),
        TypingAction::Backspace => {
          text.pop();
        }
      }
    }
    assert_eq!(text, input);
  }
}
