//! `send_async!`: the third generation mode, next to `strip_async!`.

use proc_macro2::{
    Delimiter, Group, Ident, Punct, Spacing, Span, TokenStream as TokenStream2, TokenTree,
};

fn is_ident(token: &TokenTree, name: &str) -> bool {
    matches!(token, TokenTree::Ident(ident) if *ident == name)
}

fn is_punct(token: &TokenTree, ch: char) -> bool {
    matches!(token, TokenTree::Punct(punct) if punct.as_char() == ch)
}

fn brace(token: &TokenTree) -> Option<&Group> {
    match token {
        TokenTree::Group(group) if group.delimiter() == Delimiter::Brace => Some(group),
        _ => None,
    }
}

pub(crate) fn transform(input: TokenStream2) -> TokenStream2 {
    let tokens: Vec<TokenTree> = input.into_iter().collect();
    let mut out = TokenStream2::new();
    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];
        if is_ident(token, "trait") && matches!(tokens.get(i + 1), Some(TokenTree::Ident(_))) {
            let body_at = (i..tokens.len())
                .find(|&j| brace(&tokens[j]).is_some())
                .expect("trait body");
            let body = brace(&tokens[body_at]).unwrap();
            let (new_body, marker) = transform_trait_body(body.stream());
            out.extend(add_supertraits(&tokens[i..body_at], marker));
            let mut group = Group::new(Delimiter::Brace, new_body);
            group.set_span(body.span());
            out.extend([TokenTree::Group(group)]);
            i = body_at + 1;
            continue;
        }
        match token {
            TokenTree::Group(group) => {
                let mut new = Group::new(group.delimiter(), transform(group.stream()));
                new.set_span(group.span());
                out.extend([TokenTree::Group(new)]);
            }
            _ => out.extend([token.clone()]),
        }
        i += 1;
    }
    out
}

/// Which auto traits the trait's async methods need.
#[derive(Clone, Copy, PartialEq)]
enum Marker {
    None,
    Send,
    SendSync,
}

fn add_supertraits(header: &[TokenTree], marker: Marker) -> TokenStream2 {
    if marker == Marker::None {
        return header.iter().cloned().collect();
    }
    let mut depth = 0i32;
    let mut colon = false;
    let mut where_at = header.len();
    for (k, token) in header.iter().enumerate() {
        match token {
            TokenTree::Punct(p) if p.as_char() == '<' => depth += 1,
            TokenTree::Punct(p) if p.as_char() == '>' => depth -= 1,
            TokenTree::Punct(p)
                if p.as_char() == ':'
                    && depth == 0
                    && p.spacing() == Spacing::Alone
                    && !header
                        .get(k.wrapping_sub(1))
                        .is_some_and(|t| is_punct(t, ':')) =>
            {
                colon = true
            }
            TokenTree::Ident(ident) if *ident == "where" && depth == 0 => {
                where_at = k;
                break;
            }
            _ => {}
        }
    }
    let mut bounds: Vec<TokenTree> = vec![TokenTree::Punct(Punct::new(
        if colon { '+' } else { ':' },
        Spacing::Alone,
    ))];
    bounds.extend(path(&["core", "marker", "Send"]));
    if marker == Marker::SendSync {
        bounds.push(TokenTree::Punct(Punct::new('+', Spacing::Alone)));
        bounds.extend(path(&["core", "marker", "Sync"]));
    }
    header[..where_at]
        .iter()
        .cloned()
        .chain(bounds)
        .chain(header[where_at..].iter().cloned())
        .collect()
}

fn path(segments: &[&str]) -> Vec<TokenTree> {
    let mut out = Vec::new();
    for segment in segments {
        out.push(TokenTree::Punct(Punct::new(':', Spacing::Joint)));
        out.push(TokenTree::Punct(Punct::new(':', Spacing::Alone)));
        out.push(TokenTree::Ident(Ident::new(segment, Span::call_site())));
    }
    out
}

fn transform_trait_body(input: TokenStream2) -> (TokenStream2, Marker) {
    let tokens: Vec<TokenTree> = input.into_iter().collect();
    let mut out = TokenStream2::new();
    let mut marker = Marker::None;
    let mut i = 0;
    while i < tokens.len() {
        if is_ident(&tokens[i], "async") && tokens.get(i + 1).is_some_and(|t| is_ident(t, "fn")) {
            let end = (i + 2..tokens.len())
                .find(|&j| is_punct(&tokens[j], ';') || brace(&tokens[j]).is_some())
                .expect("async fn end");
            let sig = &tokens[i + 1..end];
            if takes_shared_self(sig) {
                marker = Marker::SendSync;
            } else if marker == Marker::None {
                marker = Marker::Send;
            }
            out.extend(rewrite_signature(sig));
            match brace(&tokens[end]) {
                Some(body) => {
                    let inner: TokenStream2 = [
                        TokenTree::Ident(Ident::new("async", Span::call_site())),
                        TokenTree::Ident(Ident::new("move", Span::call_site())),
                        TokenTree::Group(body.clone()),
                    ]
                    .into_iter()
                    .collect();
                    out.extend([TokenTree::Group(Group::new(Delimiter::Brace, inner))]);
                }
                None => out.extend([tokens[end].clone()]),
            }
            i = end + 1;
            continue;
        }
        out.extend([tokens[i].clone()]);
        i += 1;
    }
    (out, marker)
}

fn takes_shared_self(sig: &[TokenTree]) -> bool {
    let Some(TokenTree::Group(params)) = sig
        .iter()
        .find(|t| matches!(t, TokenTree::Group(g) if g.delimiter() == Delimiter::Parenthesis))
    else {
        return false;
    };
    let params: Vec<TokenTree> = params.stream().into_iter().collect();
    if !params.first().is_some_and(|t| is_punct(t, '&')) {
        return false;
    }
    let mut rest = &params[1..];
    if rest.first().is_some_and(|t| is_punct(t, '\'')) {
        rest = &rest[2..];
    }
    rest.first().is_some_and(|t| is_ident(t, "self"))
}

/// `fn name<..>(..) -> R where ..` into
/// `fn name<..>(..) -> impl Future<Output = R> + Send where ..`.
fn rewrite_signature(sig: &[TokenTree]) -> TokenStream2 {
    let arrow = (0..sig.len().saturating_sub(1)).find(|&k| {
        is_punct(&sig[k], '-')
            && is_punct(&sig[k + 1], '>')
            && matches!(&sig[k], TokenTree::Punct(p) if p.spacing() == Spacing::Joint)
    });
    let (head, ret, tail) = match arrow {
        Some(k) => {
            let where_at = (k + 2..sig.len())
                .find(|&j| is_ident(&sig[j], "where"))
                .unwrap_or(sig.len());
            (
                &sig[..k],
                sig[k + 2..where_at]
                    .iter()
                    .cloned()
                    .collect::<TokenStream2>(),
                &sig[where_at..],
            )
        }
        None => {
            let where_at = sig
                .iter()
                .position(|t| is_ident(t, "where"))
                .unwrap_or(sig.len());
            let unit = TokenTree::Group(Group::new(Delimiter::Parenthesis, TokenStream2::new()));
            (
                &sig[..where_at],
                [unit].into_iter().collect(),
                &sig[where_at..],
            )
        }
    };
    let mut out: TokenStream2 = head.iter().cloned().collect();
    out.extend([
        TokenTree::Punct(Punct::new('-', Spacing::Joint)),
        TokenTree::Punct(Punct::new('>', Spacing::Alone)),
        TokenTree::Ident(Ident::new("impl", Span::call_site())),
    ]);
    out.extend(path(&["core", "future", "Future"]));
    out.extend([
        TokenTree::Punct(Punct::new('<', Spacing::Alone)),
        TokenTree::Ident(Ident::new("Output", Span::call_site())),
        TokenTree::Punct(Punct::new('=', Spacing::Alone)),
    ]);
    out.extend(ret);
    out.extend([
        TokenTree::Punct(Punct::new('>', Spacing::Alone)),
        TokenTree::Punct(Punct::new('+', Spacing::Alone)),
    ]);
    out.extend(path(&["core", "marker", "Send"]));
    out.extend(tail.iter().cloned());
    out
}

#[cfg(test)]
mod tests {
    use super::transform;

    fn expand(src: &str) -> String {
        transform(src.parse().unwrap()).to_string()
    }

    #[test]
    fn trait_methods_return_send_futures() {
        let out = expand(
            "pub trait Read: ErrorType { async fn read(&mut self, buf: &mut [u8]) -> Result<usize, E>; }",
        );
        assert!(
            out.contains("ErrorType + :: core :: marker :: Send {"),
            "{out}"
        );
        assert!(
            out.contains("fn read (& mut self , buf : & mut [u8]) -> impl :: core :: future :: Future < Output = Result < usize , E >> + :: core :: marker :: Send ;"),
            "{out}"
        );
    }

    #[test]
    fn shared_receivers_add_sync_and_bodies_become_async_blocks() {
        let out = expand("trait Fs { async fn sync(&self) { flush().await } }");
        assert!(
            out.contains("trait Fs : :: core :: marker :: Send + :: core :: marker :: Sync"),
            "{out}"
        );
        assert!(out.contains("{ async move { flush () . await } }"), "{out}");
    }

    #[test]
    fn impls_pass_through() {
        let src = "impl Read for X { async fn read(&mut self) -> usize { 0 } }";
        assert_eq!(
            expand(src),
            src.parse::<proc_macro2::TokenStream>().unwrap().to_string()
        );
    }
}
