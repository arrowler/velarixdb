use rand::{distributions::Alphanumeric, Rng};

pub fn generate_random_id(length: usize) -> String {
    let rng = rand::thread_rng();
    let id: String = rng
        .sample_iter(&Alphanumeric)
        .take(length)
        .map(char::from)
        .collect();
    id
}
