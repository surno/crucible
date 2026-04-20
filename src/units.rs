#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteSize(u64);

impl ByteSize {
    pub const fn bytes(n: u64) -> Self {
        Self(n)
    }

    pub const fn kib(n: u64) -> Self {
        Self(n * 1024)
    }

    pub const fn mib(n: u64) -> Self {
        Self(n * 1024 * 1024)
    }

    pub const fn gib(n: u64) -> Self {
        Self(n * 1024 * 1024 * 1024)
    }

    pub const fn as_bytes(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_constructors_match_byte_counts() {
        assert_eq!(ByteSize::bytes(512).as_bytes(), 512);
        assert_eq!(ByteSize::kib(1).as_bytes(), 1024);
        assert_eq!(ByteSize::mib(1).as_bytes(), 1024 * 1024);
        assert_eq!(ByteSize::gib(1).as_bytes(), 1024 * 1024 * 1024);
    }

    #[test]
    fn equal_sizes_compare_equal_regardless_of_constructor() {
        assert_eq!(ByteSize::kib(1024), ByteSize::mib(1));
        assert_eq!(ByteSize::mib(1024), ByteSize::gib(1));
    }
}
