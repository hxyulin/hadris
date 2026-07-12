# hadris-fixed

Fixed-capacity byte, UTF-8, and UTF-16 types for `no_std` applications.
Raw bytes and validated text use distinct types, so safe text APIs cannot be
constructed from invalid UTF-8.
