use std::time::Duration;

use crate::{
    Error, Token,
    auth::AuthFlow,
    client::{self, Client},
    endpoint::Endpoint,
    error::Result,
};
use serde::{Deserialize, Deserializer, de::DeserializeOwned};

pub mod album;
pub mod artist;
pub mod audio;
pub mod audiobook;
pub mod category;
pub mod market;
pub mod player;
pub mod playlist;
pub mod recommendation;
pub mod search;
pub mod show;
pub mod track;
pub mod user;

const PAGE_MAX_LIMIT: u32 = 50;
const PAGINATION_INTERVAL: Duration = Duration::from_millis(100);

/// This represents a page of items, which is a segment of data returned by the
/// Spotify API.
///
/// To get the rest of the data, the fields of this struct, or, preferably,
/// some methods can be used to get the
/// [next](Self::get_next) or [previous](Self::get_previous) page, or
/// the [remaining](Self::get_remaining) or [all](Self::get_all) items.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Page<T: Clone> {
    /// The URL to the API endpoint returning this page.
    pub href: String,
    /// The maximum amount of items in the response.
    pub limit: u32,
    /// The URL to the next page.
    /// For pagination, see [`get_next`](Self::get_next).
    pub next: Option<String>,
    /// The offset of the returned items.
    pub offset: u32,
    /// The URL to the previous page.
    /// For pagination, see [`get_previous`](Self::get_previous).
    pub previous: Option<String>,
    /// The amount of returned items.
    pub total: u32,
    /// A list of the items, which includes `null` values.
    /// To get only the `Some` values, use [`filtered_items`](Self::filtered_items).
    pub items: Vec<Option<T>>,
}

impl<T: Clone + DeserializeOwned> Page<T> {
    /// Get a list of only the `Some` values from a Page's items.
    pub fn filtered_items(&self) -> Vec<T> {
        self.items.clone().into_iter().flatten().collect()
    }

    /// Get the next page.
    ///
    /// If there is no next page, this will return an
    /// [`Error::NoRemainingPages`](crate::error::Error::NoRemainingPages).
    pub async fn get_next(&self, spotify: &Client<Token, impl AuthFlow>) -> Result<Self> {
        let Some(next) = self.next.as_ref() else {
            return Err(Error::NoRemainingPages);
        };

        // Remove `API_URL`from the string, as spotify.get()
        // (or rather spotify.request) appends it already.
        let next = next.replace(client::API_URL, "");

        spotify.get(next, [("limit", self.limit)]).await
    }

    /// Get the previous page.
    ///
    /// If there is no previous page, this will return an
    /// [`Error::NoRemainingPages`](crate::error::Error::NoRemainingPages).
    pub async fn get_previous(&self, spotify: &Client<Token, impl AuthFlow>) -> Result<Self> {
        let Some(previous) = self.previous.as_ref() else {
            return Err(Error::NoRemainingPages);
        };

        // Remove `API_URL`from the string, as spotify.get()
        // (or rather spotify.request) appends it already.
        let previous = previous.replace(client::API_URL, "");

        spotify.get(previous, [("limit", self.limit)]).await
    }

    /// Get the items of all the remaining pages - that is, all the pages found
    /// after the current one.
    pub async fn get_remaining(
        mut self,
        spotify: &Client<Token, impl AuthFlow>,
    ) -> Result<Vec<Option<T>>> {
        let mut items = std::mem::take(&mut self.items);
        self.limit = PAGE_MAX_LIMIT;
        let mut page = self;

        // Get all the next pages (if any)
        if page.next.is_some() {
            loop {
                let next_page = page.get_next(spotify).await;

                match next_page {
                    Ok(mut p) => {
                        items.append(&mut p.items);
                        page = p;
                    }

                    Err(err) => match err {
                        Error::NoRemainingPages => break,
                        _ => return Err(err),
                    },
                };

                tokio::time::sleep(PAGINATION_INTERVAL).await;
            }
        }

        Ok(items)
    }

    /// Get the items of all of the pages - that is, all the pages found both before and
    /// after the current one.
    pub async fn get_all(
        mut self,
        spotify: &Client<Token, impl AuthFlow>,
    ) -> Result<Vec<Option<T>>> {
        let mut items = std::mem::take(&mut self.items);
        self.limit = PAGE_MAX_LIMIT;

        // Get all the previous pages (if any)
        if self.previous.is_some() {
            let mut page = self.clone();

            loop {
                let previous_page = page.get_previous(spotify).await;

                match previous_page {
                    Ok(mut p) => {
                        items.append(&mut p.items);
                        page = p;
                    }
                    Err(err) => match err {
                        Error::NoRemainingPages => break,
                        _ => return Err(err),
                    },
                };

                tokio::time::sleep(PAGINATION_INTERVAL).await;
            }
        }

        // Get all the next pages (if any)
        if self.next.is_some() {
            let mut page = self;

            loop {
                let next_page = page.get_next(spotify).await;

                match next_page {
                    Ok(mut p) => {
                        items.append(&mut p.items);
                        page = p;
                    }

                    Err(err) => match err {
                        Error::NoRemainingPages => break,
                        _ => return Err(err),
                    },
                };

                tokio::time::sleep(PAGINATION_INTERVAL).await;
            }
        }

        Ok(items)
    }
}

/// This represents a page of items, which is a segment of data returned by the
/// Spotify API.
///
/// It's similar to [`Page`], except that it uses a different approach for
/// pagination - instead of using a `next` and `previous` field to get another
/// page, it uses a Unix timestamp (in miliseconds).
///
/// To get the rest of the data, the `cursors` field (and others), or, preferably,
/// the [get_before](Self::get_before) and [get_after](Self::get_after) methods can be used.
// (and possibly in other situations)
// it happens because some fields are null when they shouldn't be
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct CursorPage<T: Clone, E: Endpoint + Default> {
    /// The URL to the API endpoint returning this page.
    pub href: String,
    /// The maximum amount of items in the response.
    pub limit: u32,
    /// The URL to the next page.
    pub next: Option<String>,
    /// The cursor object used to get the previous/next page.
    pub cursors: Option<Cursor>,
    /// The amount of returned items.
    pub total: Option<u32>,
    /// A list of the items, which includes `null` values.
    pub items: Vec<Option<T>>,
    // Used to keep track of which endpoint should be called to
    // get subsequent pages.
    #[serde(skip)]
    endpoint: E,
}

impl<T: Clone + DeserializeOwned, E: Endpoint + Default + Clone> CursorPage<T, E> {
    /// Get a list of only the `Some` values from a Cursor Page's items.
    pub fn filtered_items(&self) -> Vec<T> {
        self.items.clone().into_iter().flatten().collect()
    }

    /// Get the page chronologically before the current one.
    ///
    /// If there is no previous page, this will return an
    /// [`Error::NoRemainingPages`](crate::error::Error::NoRemainingPages).
    pub async fn get_before(&self, spotify: &Client<Token, impl AuthFlow>) -> Result<Self> {
        let Some(ref cursors) = self.cursors else {
            return Err(Error::NoRemainingPages);
        };

        let Some(before) = cursors.before.as_ref() else {
            return Err(Error::NoRemainingPages);
        };

        spotify
            .get(
                self.endpoint.endpoint_url().to_owned(),
                [("before", before), ("limit", &(self.limit.to_string()))],
            )
            .await
    }

    /// Get the page chronologically after the current one.
    ///
    /// If there is no next page, this will return an
    /// [`Error::NoRemainingPages`](crate::error::Error::NoRemainingPages).
    pub async fn get_after(&self, spotify: &Client<Token, impl AuthFlow>) -> Result<Self> {
        let Some(ref cursors) = self.cursors else {
            return Err(Error::NoRemainingPages);
        };

        let Some(after) = cursors.after.as_ref() else {
            return Err(Error::NoRemainingPages);
        };

        spotify
            .get(
                self.endpoint.endpoint_url().to_owned(),
                [("after", after), ("limit", &(self.limit.to_string()))],
            )
            .await
    }

    /// Get the items of all the remaining pages - that is, all the pages found
    /// after the current one.
    pub async fn get_remaining(
        mut self,
        spotify: &Client<Token, impl AuthFlow>,
    ) -> Result<Vec<Option<T>>> {
        let mut items = std::mem::take(&mut self.items);
        self.limit = PAGE_MAX_LIMIT;
        let mut page = self;

        // Get all the next pages (if any)
        if let Some(ref cursors) = page.cursors {
            if cursors.after.is_some() {
                loop {
                    let next_page = page.get_after(spotify).await;

                    match next_page {
                        Ok(mut p) => {
                            items.append(&mut p.items);
                            page = p;
                        }
                        Err(err) => match err {
                            Error::NoRemainingPages => break,
                            _ => return Err(err),
                        },
                    }

                    tokio::time::sleep(PAGINATION_INTERVAL).await;
                }
            }
        }

        Ok(items)
    }

    pub async fn get_all(
        mut self,
        spotify: &Client<Token, impl AuthFlow>,
    ) -> Result<Vec<Option<T>>> {
        let mut items = std::mem::take(&mut self.items);
        self.limit = PAGE_MAX_LIMIT;

        // Get all the previous pages (if any)
        if let Some(ref cursors) = self.cursors {
            if cursors.before.is_some() {
                let mut page = self.clone();

                loop {
                    let previous_page = page.get_before(spotify).await;

                    match previous_page {
                        Ok(mut p) => {
                            items.append(&mut p.items);
                            page = p;
                        }
                        Err(err) => match err {
                            Error::NoRemainingPages => break,
                            _ => return Err(err),
                        },
                    }

                    tokio::time::sleep(PAGINATION_INTERVAL).await;
                }
            }
        }

        // Get all the next pages (if any)
        if let Some(ref cursors) = self.cursors {
            if cursors.after.is_some() {
                let mut page = self;

                loop {
                    let next_page = page.get_after(spotify).await;

                    match next_page {
                        Ok(mut p) => {
                            items.append(&mut p.items);
                            page = p;
                        }
                        Err(err) => match err {
                            Error::NoRemainingPages => break,
                            _ => return Err(err),
                        },
                    }

                    tokio::time::sleep(PAGINATION_INTERVAL).await;
                }
            }
        }

        Ok(items)
    }
}

/// A cursor used to paginate results returned as a [`CursorPage`].
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Cursor {
    pub after: Option<String>,
    pub before: Option<String>,
}

/// An image used in various contexts.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Image {
    /// The URL of the image.
    pub url: String,
    /// The height in pixels.
    pub height: Option<u32>,
    /// The width in pixels.
    pub width: Option<u32>,
}

/// A copyright statement.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Copyright {
    /// The copyright text.
    pub text: String,
    /// The copyright type.
    pub r#type: CopyrightType,
}

/// A content restriction.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Restriction {
    /// The reason for the restriction.
    pub reason: RestrictionReason,
}

/// Contains known external IDs for content.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ExternalIds {
    /// The [International Standard Recording Code](https://en.wikipedia.org/wiki/International_Standard_Recording_Code)
    /// for the content.
    pub isrc: Option<String>,
    /// The [International Article Number](https://en.wikipedia.org/wiki/International_Article_Number)
    /// for the content.
    pub ean: Option<String>,
    /// The [Universal Product Code](https://en.wikipedia.org/wiki/Universal_Product_Code)
    /// for the content.
    pub upc: Option<String>,
}

/// Contains external URLs for content. Currently, it seems that only Spotify
/// URLs are included here.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ExternalUrls {
    /// The [Spotify URL](https://developer.spotify.com/documentation/web-api/concepts/spotify-uris-ids)
    /// for the content.
    pub spotify: String,
}

/// Information about the followers of an artist, playlist or user.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Followers {
    /// This will always be set to null, as the Web API does not support it at the moment.
    pub href: Option<String>,
    /// The total amount of followers.
    pub total: u32,
}

/// The user's latest position in a chapter or episode.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ResumePoint {
    /// Whether or not the chapter or episode has fully been played by the user.
    pub fully_played: bool,
    /// The user's latest position in miliseconds.
    pub resume_position_ms: u32,
}

/// The reason for restriction on content.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RestrictionReason {
    /// A restriction set because of the market of a user.
    Market,
    /// A restriction set because of the user's subscription type.
    Product,
    /// A restriction set because the content is explicit, and the user settings
    /// are set so that explicit conent can't be played.
    Explicit,
    #[serde(other)]
    /// Any other type of restriction, as more may be added in the future.
    Unknown,
}

/// The copyright type for a piece of content:
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub enum CopyrightType {
    #[serde(rename = "C")]
    /// The copyright.
    Copyright,
    #[serde(rename = "P")]
    /// The sound recording (performance) copyright.
    Performance,
}

/// The precision with which a date is known.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DatePrecision {
    /// The date is known at the year level.
    Year,
    /// The date is known at the month level.
    Month,
    /// The date is known at the day level.
    Day,
}

/// An item that can be played.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
#[non_exhaustive]
pub enum PlayableItem {
    /// A Spotify track (song).
    Track(track::Track),
    /// An episode of a show.
    Episode(show::Episode),
    /// Any other type of PlayableItem, as more may be added in the future.
    Unknown(serde_json::Value),
}

// A function to convert a "null" JSON value to the default of given type,
// to make the API slightly nicer to use for people.
fn null_to_default<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    T: Default + Deserialize<'de>,
    D: Deserializer<'de>,
{
    Ok(Option::deserialize(deserializer)?.unwrap_or_default())
}
