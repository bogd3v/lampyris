use serde::{Deserialize, Serialize};

/// A public post, already reduced to what the detector needs.
///
/// Sources (Bluesky Jetstream today, others later) map their own formats into this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Post {
    /// Stable author identifier (a Bluesky DID, a Mastodon account URI, ...).
    pub author: String,
    /// Identifier of the post within the author's repository (Bluesky rkey).
    pub id: String,
    /// Event time in microseconds since the Unix epoch. The detector only ever
    /// uses this clock, never the wall clock, so replays are deterministic.
    pub time_us: i64,
    /// Raw post text.
    pub text: String,
    /// Language tags declared by the client, if any (e.g. `["es"]`).
    #[serde(default)]
    pub langs: Vec<String>,
    /// Whether the post embeds an external link card. Many accounts sharing the
    /// same news headline is a common *legitimate* source of identical text, so
    /// this is reported as a signal rather than hidden.
    #[serde(default)]
    pub has_external_link: bool,
}

impl Post {
    pub fn new(
        author: impl Into<String>,
        id: impl Into<String>,
        time_us: i64,
        text: impl Into<String>,
    ) -> Self {
        Self {
            author: author.into(),
            id: id.into(),
            time_us,
            text: text.into(),
            langs: Vec::new(),
            has_external_link: false,
        }
    }

    /// Reference to this post that can be stored, shared and re-verified later.
    pub fn to_ref(&self) -> PostRef {
        PostRef {
            author: self.author.clone(),
            id: self.id.clone(),
            time_us: self.time_us,
        }
    }
}

/// A pointer to a public post. Reports carry these instead of copying content,
/// so anyone can re-fetch and re-verify the evidence from the original network.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PostRef {
    pub author: String,
    pub id: String,
    pub time_us: i64,
}

impl PostRef {
    /// AT-URI for Bluesky posts (`at://did/app.bsky.feed.post/rkey`).
    pub fn at_uri(&self) -> String {
        format!("at://{}/app.bsky.feed.post/{}", self.author, self.id)
    }

    /// Public web URL for Bluesky posts.
    pub fn bsky_url(&self) -> String {
        format!("https://bsky.app/profile/{}/post/{}", self.author, self.id)
    }
}
