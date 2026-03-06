//! Proc macros for dual sync/async code generation in Hadris.
//!
//! Provides `strip_async!` which removes `async`/`.await` from token streams,
//! enabling the same source to compile as both sync and async code.

extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use proc_macro2::TokenTree;

/// Strips all `async` keywords and `.await` expressions from the input.
///
/// This macro walks the token stream and:
/// 1. Removes `async` keyword before `fn`
/// 2. Removes `async` keyword before `move` (closures/blocks)
/// 3. Removes `.await` expressions (the `.` and `await` tokens)
/// 4. Passes everything else through unchanged
///
/// Used inside `sync` modules via `io_transform!` to generate synchronous
/// versions of async source code.
#[proc_macro]
pub fn strip_async(input: TokenStream) -> TokenStream {
    let input = TokenStream2::from(input);
    let output = strip_async_from_stream(input);
    TokenStream::from(output)
}

fn strip_async_from_stream(input: TokenStream2) -> TokenStream2 {
    let mut output = TokenStream2::new();
    let mut iter = input.into_iter().peekable();

    while let Some(token) = iter.next() {
        match &token {
            TokenTree::Ident(ident) if *ident == "async" => {
                // Look ahead: if next is `fn` or `move`, skip the `async`
                if let Some(next) = iter.peek()
                    && let TokenTree::Ident(next_ident) = next
                {
                    let s = next_ident.to_string();
                    if s == "fn" || s == "move" || s == "unsafe" {
                        // Skip `async`, let the next token be emitted normally
                        continue;
                    }
                }
                // `async` not followed by `fn`/`move`/`unsafe` — keep it
                // (shouldn't normally happen in our codebase, but be safe)
                output.extend(core::iter::once(token));
            }
            TokenTree::Punct(punct) if punct.as_char() == '.' => {
                // Check if next token is `await`
                if let Some(next) = iter.peek()
                    && let TokenTree::Ident(ident) = next
                    && *ident == "await"
                {
                    // Skip both `.` and `await`
                    iter.next();
                    continue;
                }
                // Not `.await`, keep the `.`
                output.extend(core::iter::once(token));
            }
            TokenTree::Group(group) => {
                // Recurse into groups (braces, parens, brackets)
                let inner = strip_async_from_stream(group.stream());
                let mut new_group = proc_macro2::Group::new(group.delimiter(), inner);
                new_group.set_span(group.span());
                output.extend(core::iter::once(TokenTree::Group(new_group)));
            }
            _ => {
                output.extend(core::iter::once(token));
            }
        }
    }

    output
}
