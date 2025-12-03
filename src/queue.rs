//! Thread-safe bounded queue for buffering output data.

use std::collections::VecDeque;
use tokio::sync::Mutex;

use crate::error::Error;

/// A thread-safe bounded queue that supports appending and retrieving elements.
#[derive(Debug)]
pub struct BoundedQueue<T> {
    buffer: Mutex<VecDeque<T>>,
    capacity: usize,
}

impl<T: Clone> BoundedQueue<T> {
    /// Creates a new bounded queue with the specified capacity.
    ///
    /// # Panics
    ///
    /// Panics if capacity is 0.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "capacity must be greater than 0");
        Self {
            buffer: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
        }
    }

    /// Appends elements to the end of the queue.
    ///
    /// Returns an error if adding the elements would exceed the capacity.
    pub async fn append(&self, elements: Vec<T>) -> Result<(), Error> {
        if elements.is_empty() {
            return Ok(());
        }

        let mut buffer = self.buffer.lock().await;
        if buffer.len() + elements.len() > self.capacity {
            return Err(Error::QueueFull);
        }

        buffer.extend(elements);
        Ok(())
    }

    /// Tries to append as many elements as possible without exceeding capacity.
    ///
    /// Returns the number of elements actually added.
    pub async fn try_append(&self, elements: Vec<T>) -> usize {
        if elements.is_empty() {
            return 0;
        }

        let mut buffer = self.buffer.lock().await;
        let available = self.capacity.saturating_sub(buffer.len());
        let to_add = elements.len().min(available);

        buffer.extend(elements.into_iter().take(to_add));
        to_add
    }

    /// Retrieves up to `size` elements from the beginning of the queue.
    ///
    /// Returns fewer elements if the queue has less than `size` elements.
    pub async fn get(&self, size: usize) -> Vec<T> {
        let mut buffer = self.buffer.lock().await;
        let to_take = size.min(buffer.len());
        buffer.drain(..to_take).collect()
    }

    /// Retrieves all available elements from the queue.
    pub async fn get_all(&self) -> Vec<T> {
        let mut buffer = self.buffer.lock().await;
        buffer.drain(..).collect()
    }

    /// Returns the current number of elements in the queue.
    pub async fn len(&self) -> usize {
        self.buffer.lock().await.len()
    }

    /// Returns true if the queue is empty.
    pub async fn is_empty(&self) -> bool {
        self.buffer.lock().await.is_empty()
    }

    /// Returns true if the queue is at maximum capacity.
    pub async fn is_full(&self) -> bool {
        self.buffer.lock().await.len() >= self.capacity
    }

    /// Returns the number of elements that can still be added.
    pub async fn available(&self) -> usize {
        self.capacity.saturating_sub(self.buffer.lock().await.len())
    }

    /// Clears all elements from the queue.
    pub async fn clear(&self) {
        self.buffer.lock().await.clear();
    }

    /// Returns the maximum capacity of the queue.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Peeks at up to `size` elements without removing them.
    pub async fn peek(&self, size: usize) -> Vec<T> {
        let buffer = self.buffer.lock().await;
        let to_take = size.min(buffer.len());
        buffer.iter().take(to_take).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_basic_operations() {
        let queue = BoundedQueue::<i32>::new(10);

        assert!(queue.is_empty().await);
        assert_eq!(queue.len().await, 0);

        queue.append(vec![1, 2, 3]).await.unwrap();
        assert_eq!(queue.len().await, 3);
        assert!(!queue.is_empty().await);

        let items = queue.get(2).await;
        assert_eq!(items, vec![1, 2]);
        assert_eq!(queue.len().await, 1);

        let items = queue.get_all().await;
        assert_eq!(items, vec![3]);
        assert!(queue.is_empty().await);
    }

    #[tokio::test]
    async fn test_capacity() {
        let queue = BoundedQueue::<i32>::new(5);

        queue.append(vec![1, 2, 3]).await.unwrap();
        assert!(queue.append(vec![4, 5, 6]).await.is_err());

        queue.append(vec![4, 5]).await.unwrap();
        assert!(queue.is_full().await);
    }

    #[tokio::test]
    async fn test_try_append() {
        let queue = BoundedQueue::<i32>::new(5);

        queue.append(vec![1, 2, 3]).await.unwrap();
        let added = queue.try_append(vec![4, 5, 6, 7]).await;
        assert_eq!(added, 2);
        assert!(queue.is_full().await);
    }

    #[tokio::test]
    async fn test_peek() {
        let queue = BoundedQueue::<i32>::new(10);
        queue.append(vec![1, 2, 3]).await.unwrap();

        let peeked = queue.peek(2).await;
        assert_eq!(peeked, vec![1, 2]);
        assert_eq!(queue.len().await, 3); // Not removed
    }
}

