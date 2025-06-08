use proc_macro::{TokenStream, TokenTree};

pub(crate) trait SpecialIdentReplacer<T> {
    fn replace(&self, ident: proc_macro::Ident, data: T) -> Option<TokenStream>;
}

pub(crate) fn replace<T: Clone>(
    input: TokenStream,
    data_iterator: impl Iterator<Item = T> + Clone,
    replacer: impl SpecialIdentReplacer<T> + Clone,
) -> TokenStream {
    let mut special = None;
    input
        .into_iter()
        .flat_map(move |token| match token {
            TokenTree::Group(group) => {
                if special.is_some() && group.delimiter() == proc_macro::Delimiter::Brace {
                    special = None;
                    data_iterator
                        .clone()
                        .flat_map(|data| replace_inner(group.stream(), data, replacer.clone()))
                        .collect::<TokenStream>()
                        .into_iter()
                } else {
                    let stream = replace(group.stream(), data_iterator.clone(), replacer.clone());

                    // TODO: preserve group delimiter span
                    let new_group = proc_macro::Group::new(group.delimiter(), stream);

                    if let Some(special) = std::mem::take(&mut special) {
                        [TokenTree::Punct(special), TokenTree::Group(new_group)]
                            .into_iter()
                            .collect::<TokenStream>()
                            .into_iter()
                    } else {
                        TokenStream::from(TokenTree::Group(new_group)).into_iter()
                    }
                }
            }
            TokenTree::Punct(punct) if punct.as_char() == '#' => {
                if let Some(original_special) = std::mem::take(&mut special) {
                    // Return the previous special character and consume this special character
                    special = Some(punct);
                    TokenStream::from(TokenTree::Punct(original_special)).into_iter()
                } else {
                    // Consume this special character
                    special = Some(punct);
                    TokenStream::new().into_iter()
                }
            }
            token => {
                if let Some(original_special) = std::mem::take(&mut special) {
                    // Return the previous special character along with this token
                    [TokenTree::Punct(original_special), token]
                        .into_iter()
                        .collect::<TokenStream>()
                        .into_iter()
                } else {
                    // Return this token
                    TokenStream::from(token).into_iter()
                }
            }
        })
        .collect()
}

pub(crate) fn replace_inner<T: Clone>(
    input: TokenStream,
    data: T,
    replacer: impl SpecialIdentReplacer<T> + Clone,
) -> impl Iterator<Item = TokenTree> {
    let mut special = None;
    input.into_iter().flat_map(move |token| match token {
        TokenTree::Ident(ident) if special.is_some() => {
            if let Some(stream) = replacer.replace(ident.clone(), data.clone()) {
                special = None;
                stream.into_iter()
            } else {
                [
                    TokenTree::Punct(std::mem::take(&mut special).unwrap()),
                    TokenTree::Ident(ident),
                ]
                .into_iter()
                .collect::<TokenStream>()
                .into_iter()
            }
        }
        TokenTree::Group(group) => {
            let stream = replace_inner(group.stream(), data.clone(), replacer.clone())
                .collect::<TokenStream>();

            // TODO: preserve group delimiter span
            let new_group = proc_macro::Group::new(group.delimiter(), stream);

            if let Some(special) = std::mem::take(&mut special) {
                [TokenTree::Punct(special), TokenTree::Group(new_group)]
                    .into_iter()
                    .collect::<TokenStream>()
                    .into_iter()
            } else {
                TokenStream::from(TokenTree::Group(new_group)).into_iter()
            }
        }
        TokenTree::Punct(punct) if punct.as_char() == '#' => {
            if let Some(original_special) = std::mem::take(&mut special) {
                // Return the previous special character and consume this special character
                special = Some(punct);
                TokenStream::from(TokenTree::Punct(original_special)).into_iter()
            } else {
                // Consume this special character
                special = Some(punct);
                TokenStream::new().into_iter()
            }
        }
        token => {
            if let Some(original_special) = std::mem::take(&mut special) {
                // Return the previous special character along with this token
                [TokenTree::Punct(original_special), token]
                    .into_iter()
                    .collect::<TokenStream>()
                    .into_iter()
            } else {
                // Return this token
                TokenStream::from(token).into_iter()
            }
        }
    })
}
