use std::collections::HashSet;

// Off Windows there is no process table or Start Menu to scan. The catalogue
// still surfaces curated apps plus whatever the audio backend reports as
// playing, which keeps the picker useful when running against the fake backend.
pub fn running_processes() -> HashSet<String> {
    HashSet::new()
}

pub fn installed_applications() -> Vec<(String, String)> {
    Vec::new()
}
