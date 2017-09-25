//! Iterator over database spaces

use error::{ErrorKind, Result};
use field::{self, field_size, Header};
use field::iterator::FieldIterator;

macro_rules! try_next {
	($t: expr) => {
		match $t {
			Ok(ok) => ok,
			Err(err) => return Some(Err(err.into())),
		}
	}
}

#[derive(Debug, PartialEq, Clone)]
pub struct OccupiedSpace<'a> {
	/// Offset from the begining of iteration slice
	pub offset: usize,
	pub data: &'a [u8],
}

#[derive(Debug, PartialEq, Clone)]
pub struct EmptySpace {
	pub offset: usize,
	pub len: usize,
}

#[derive(Debug, PartialEq, Clone)]
pub enum Space<'a> {
	Occupied(OccupiedSpace<'a>),
	Empty(EmptySpace),
}

#[derive(Debug)]
pub struct SpaceIterator<'a> {
	data: &'a [u8],
	field_body_size: usize,
	offset: usize,
	peeked: Option<Result<Space<'a>>>,
}

impl<'a> SpaceIterator<'a> {
	pub fn new(data: &'a [u8], field_body_size: usize, offset: usize) -> Self {
		SpaceIterator {
			data,
			field_body_size,
			offset,
			peeked: None,
		}
	}

	/// Move iterator forward
	pub fn move_offset_forward(&mut self, offset: usize) {
		if offset > self.offset {
			self.offset = offset;
			self.peeked = None;
		}
	}

	/// Peek next value
	pub fn peek(&mut self) -> Option<&Result<Space<'a>>> {
		if self.peeked.is_none() {
			self.peeked = self.next();
		}

		self.peeked.as_ref()
	}
}

impl<'a> Iterator for SpaceIterator<'a> {
	type Item = Result<Space<'a>>;

	fn next(&mut self) -> Option<Self::Item> {
		if let Some(peeked) = self.peeked.take() {
			return Some(peeked)
		}

		if self.data[self.offset..].is_empty() {
			return None;
		}

		let mut prev_header = None;
		let mut start = self.offset;
		let field_size = field_size(self.field_body_size);
		let mut inner = try_next!(FieldIterator::new(&self.data[self.offset..], self.field_body_size));
		while let Some(field) = inner.next() {
			let header = try_next!(field.header());
			match header {
				Header::Continued => match prev_header {
					// ommit continued fields at the beginning
					None => {
						start += field_size;
						self.offset += field_size;
						continue;
					},
					Some(Header::Inserted) | Some(Header::Continued) => {
						self.offset += field_size;
					},
					Some(Header::Deleted) | Some(Header::Uninitialized) => {
						return Some(Err(ErrorKind::Field(field::ErrorKind::InvalidHeader).into()))
					},
				},
				Header::Inserted => match prev_header {
					Some(Header::Inserted) => return Some(Ok(Space::Occupied(OccupiedSpace {
						offset: start,
						data: &self.data[start..self.offset],
					}))),
					Some(Header::Continued) | None => {
						self.offset += field_size;
					}
					// this one is unreachable
					Some(Header::Deleted) | Some(Header::Uninitialized) => return Some(Ok(Space::Empty(EmptySpace {
						offset: start,
						len: self.offset - start,
					}))),
				},
				Header::Deleted | Header::Uninitialized => match prev_header {
					// inserted is unreachable
					Some(Header::Inserted) | Some(Header::Continued) => return Some(Ok(Space::Occupied(OccupiedSpace {
						offset: start,
						data: &self.data[start..self.offset],
					}))),
					Some(Header::Deleted) | Some(Header::Uninitialized) => return Some(Ok(Space::Empty(EmptySpace {
						offset: start,
						len: self.offset - start,
					}))),
					None => {
						self.offset += field_size;
					},
				}
			}
			prev_header = Some(header);
		}

		prev_header.map(|header| match header {
			Header::Inserted | Header::Continued => Ok(Space::Occupied(OccupiedSpace {
				offset: start,
				data: &self.data[start..self.offset],
			})),
			Header::Deleted | Header::Uninitialized => Ok(Space::Empty(EmptySpace {
				offset: start,
				len: self.offset - start,
			})),
		})
	}
}

#[cfg(test)]
mod tests {
	use super::{SpaceIterator, Space, EmptySpace, OccupiedSpace};

	#[test]
	fn test_empty_space_iterator() {
		let data = &[];
		let field_body_size = 3;
		let offset = 0;

		let mut iterator = SpaceIterator::new(data, field_body_size, offset);
		assert!(iterator.next().is_none());
	}

	#[test]
	fn test_space_iterator_one_uninitialized_element() {
		let data = &[0, 1, 1, 1];
		let field_body_size = 3;
		let offset = 0;

		let first_elem = Space::Empty(EmptySpace { offset, len: 4 });
		let mut iterator = SpaceIterator::new(data, field_body_size, offset);
		assert_eq!(first_elem, iterator.next().unwrap().unwrap());
		assert!(iterator.next().is_none());
	}

	#[test]
	fn test_space_iterator_one_initialized_element() {
		let data = &[1, 1, 1, 1];
		let field_body_size = 3;
		let offset = 0;

		let first_elem = Space::Occupied(OccupiedSpace { offset, data });
		let mut iterator = SpaceIterator::new(data, field_body_size, offset);
		assert_eq!(first_elem, iterator.next().unwrap().unwrap());
		assert!(iterator.next().is_none());
	}

	#[test]
	fn test_space_iterator_two_different_spaces1() {
		let data = &[1, 1, 1, 1, 0, 0, 0, 0];
		let field_body_size = 3;
		let offset = 0;

		let first_elem = Space::Occupied(OccupiedSpace { offset, data: &data[0..4] });
		let second_elem = Space::Empty(EmptySpace { offset: offset + 4, len: 4 });
		let mut iterator = SpaceIterator::new(data, field_body_size, offset);
		assert_eq!(first_elem, iterator.next().unwrap().unwrap());
		assert_eq!(second_elem, iterator.next().unwrap().unwrap());
		assert!(iterator.next().is_none());
	}

	#[test]
	fn test_space_iterator_two_different_spaces2() {
		let data = &[0, 0, 0, 0, 1, 0, 0, 0];
		let field_body_size = 3;
		let offset = 0;

		let first_elem = Space::Empty(EmptySpace { offset, len: 4 });
		let second_elem = Space::Occupied(OccupiedSpace { offset: offset + 4, data: &data[4..8] });
		let mut iterator = SpaceIterator::new(data, field_body_size, offset);
		assert_eq!(first_elem, iterator.next().unwrap().unwrap());
		assert_eq!(second_elem, iterator.next().unwrap().unwrap());
		assert!(iterator.next().is_none());
	}

	#[test]
	fn test_space_iterator_two_inserts() {
		let data = &[1, 0, 0, 0, 1, 2, 2, 2];
		let field_body_size = 3;
		let offset = 0;

		let first_elem = Space::Occupied(OccupiedSpace { offset, data: &data[0..4] });
		let second_elem = Space::Occupied(OccupiedSpace { offset: 4, data: &data[4..8] });
		let mut iterator = SpaceIterator::new(data, field_body_size, offset);
		assert_eq!(first_elem, iterator.next().unwrap().unwrap());
		assert_eq!(second_elem, iterator.next().unwrap().unwrap());
		assert!(iterator.next().is_none());
	}

	#[test]
	fn test_space_iterator_one_long_space1() {
		let data = &[1, 0, 0, 0, 2, 0, 0, 0];
		let field_body_size = 3;
		let offset = 0;

		let first_elem = Space::Occupied(OccupiedSpace { offset, data });
		let mut iterator = SpaceIterator::new(data, field_body_size, offset);
		assert_eq!(first_elem, iterator.next().unwrap().unwrap());
		assert!(iterator.next().is_none());
	}

	#[test]
	fn test_space_iterator_one_long_space2() {
		let data = &[0, 0, 0, 0, 0, 0, 0, 0];
		let field_body_size = 3;
		let offset = 0;

		let first_elem = Space::Empty(EmptySpace { offset, len: 4 });
		let second_elem = Space::Empty(EmptySpace { offset: 4, len: 4 });
		let mut iterator = SpaceIterator::new(data, field_body_size, offset);
		assert_eq!(first_elem, iterator.next().unwrap().unwrap());
		assert_eq!(second_elem, iterator.next().unwrap().unwrap());
		assert!(iterator.next().is_none());
	}

	#[test]
	fn test_space_iterator_start_from_continued1() {
		let data = &[2, 0, 0, 0, 0, 0, 0, 0];
		let field_body_size = 3;
		let offset = 0;

		let first_elem = Space::Empty(EmptySpace { offset: 4, len: 4 });
		let mut iterator = SpaceIterator::new(data, field_body_size, offset);
		assert_eq!(first_elem, iterator.next().unwrap().unwrap());
		assert!(iterator.next().is_none());
	}

	#[test]
	fn test_space_iterator_start_from_continued2() {
		let data = &[
			2, 0, 0, 0,
			2, 0, 0, 0,
			0, 0, 0, 0
		];
		let field_body_size = 3;
		let offset = 0;

		let first_elem = Space::Empty(EmptySpace { offset: 8, len: 4 });
		let mut iterator = SpaceIterator::new(data, field_body_size, offset);
		assert_eq!(first_elem, iterator.next().unwrap().unwrap());
		assert!(iterator.next().is_none());
	}

	#[test]
	fn test_space_iterator_continued_error() {
		let data = &[0, 0, 0, 0, 2, 0, 0, 0];
		let field_body_size = 3;
		let offset = 0;

		let mut iterator = SpaceIterator::new(data, field_body_size, offset);
		assert!(iterator.next().unwrap().is_err());
	}
}