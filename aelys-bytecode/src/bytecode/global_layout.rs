use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug)]
pub struct GlobalLayout {
    id: usize,
    names: Vec<String>,
}

static GLOBAL_LAYOUT_ID: AtomicUsize = AtomicUsize::new(1);
static GLOBAL_LAYOUT_CACHE: OnceLock<Mutex<HashMap<Vec<String>, Arc<GlobalLayout>>>> =
    OnceLock::new();
static EMPTY_LAYOUT: OnceLock<Arc<GlobalLayout>> = OnceLock::new();

impl GlobalLayout {
    pub fn new(names: Vec<String>) -> Arc<Self> {
        if names.is_empty() {
            return Self::empty();
        }
        let cache = GLOBAL_LAYOUT_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        let mut guard = cache.lock().unwrap();
        if let Some(existing) = guard.get(&names) {
            return Arc::clone(existing);
        }
        let id = GLOBAL_LAYOUT_ID.fetch_add(1, Ordering::Relaxed);
        let layout = Arc::new(Self {
            id,
            names: names.clone(),
        });
        guard.insert(names, Arc::clone(&layout));
        layout
    }

    pub fn empty() -> Arc<Self> {
        EMPTY_LAYOUT
            .get_or_init(|| {
                Arc::new(Self {
                    id: 0,
                    names: Vec::new(),
                })
            })
            .clone()
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }
}
