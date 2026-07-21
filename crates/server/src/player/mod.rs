#[derive(Clone, Debug)]
pub struct Player {
    pub id: u64,
    pub name: String,
    pub address: String,
}

impl Player {
    pub fn new(id: u64, name: String, address: String) -> Self {
        Self { id, name, address }
    }
}
