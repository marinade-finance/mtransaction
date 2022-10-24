use std::collections::HashMap;

#[derive(Default)]
pub struct Bundler {
    queue: HashMap<String, Vec<(String, String)>>,
}

impl Bundler {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn enqueue(&mut self, tpu: &String, record: (String, String)) -> usize {
        match self.queue.get_mut(tpu) {
            Some(q) => {
                q.push(record);
                q.len()
            }
            _ => {
                self.queue.insert(tpu.clone(), vec![record]);
                1
            }
        }
    }

    pub fn drain(&mut self, tpu: &String) -> Option<Vec<(String, String)>> {
        let result = match self.queue.get_mut(tpu) {
            Some(q) => Some(q.drain(..).collect()),
            _ => None,
        };
        self.queue.remove(tpu);

        result
    }

    pub fn get_tpus(&self) -> Vec<String> {
        self.queue.keys().cloned().collect()
    }
}
