pub struct NonEmpty<T> {
    pub head: T,
    pub rest: Vec<T>,
}

impl<T> NonEmpty<T> {
    pub fn new(head: T) -> Self {
        Self { head, rest: vec![] }
    }

    pub fn push(&mut self, item: T) {
        self.rest.push(item);
    }

    pub fn last(&self) -> &T {
        self.rest.last().unwrap_or(&self.head)
    }

    pub fn last_mut(&mut self) -> &mut T {
        self.rest.last_mut().unwrap_or(&mut self.head)
    }

    pub fn rest(&self) -> &[T] {
        &self.rest
    }

    pub fn rest_len(&self) -> usize {
        self.rest.len()
    }
}
