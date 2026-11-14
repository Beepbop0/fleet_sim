use std::ops::{Index, IndexMut};
use thiserror::Error;

pub struct Square {
    n: usize,
    inner: Vec<usize>,
}

#[derive(Error, Debug)]
pub enum SquareError {
    #[error("columns {columns} does not equal rows {rows}")]
    NotASquare { columns: usize, rows: usize },
    #[error("index {0}{0} is not zero")]
    SelfCellNotZero(usize),
}

impl TryFrom<Vec<Vec<usize>>> for Square {
    type Error = SquareError;

    fn try_from(value: Vec<Vec<usize>>) -> Result<Self, Self::Error> {
        let n = value.len();
        let mut inner = Vec::with_capacity(n * n);
        for (i, row) in value.into_iter().enumerate() {
            if row.len() != n {
                return Err(SquareError::NotASquare {
                    rows: n,
                    columns: row.len(),
                });
            }
            for (j, item) in row.into_iter().enumerate() {
                if i == j && item != 0 {
                    return Err(SquareError::SelfCellNotZero(i));
                }
                inner.push(item);
            }
        }

        Ok(Self { n, inner })
    }
}

impl Square {
    pub fn zeros(n: usize) -> Self {
        Self {
            inner: vec![0; n * n],
            n,
        }
    }

    pub fn n(&self) -> usize {
        self.n
    }

    pub fn context(&self, arg: &str) -> Square {
        todo!()
    }
}

impl IndexMut<usize> for Square {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        let start = index * self.n;
        let end = start + self.n;
        &mut self.inner[start..end]
    }
}

impl Index<usize> for Square {
    type Output = [usize];

    fn index(&self, index: usize) -> &Self::Output {
        let start = index * self.n;
        let end = start + self.n;
        &self.inner[start..end]
    }
}
