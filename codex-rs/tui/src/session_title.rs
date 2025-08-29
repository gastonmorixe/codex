use std::sync::Mutex;
use std::sync::OnceLock;

static TITLE_CELL: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn cell() -> &'static Mutex<Option<String>> {
    TITLE_CELL.get_or_init(|| Mutex::new(None))
}

pub fn set(title: String) {
    if let Ok(mut guard) = cell().lock() {
        *guard = Some(title);
    }
}

pub fn get() -> Option<String> {
    cell().lock().ok().and_then(|g| g.clone())
}
