use serde::{Deserialize, Serialize};

pub trait SnapshotAware {
    type State: Serialize + for<'de> Deserialize<'de>;

    fn snapshot(&self) -> Self::State;
    fn restore(&mut self, state: Self::State);
}