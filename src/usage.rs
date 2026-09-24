use serde::{Deserialize, Serialize};

/// Provider-reported token counts observed during one API request.
/// Fields remain optional because many OpenAI-compatible gateways omit usage
/// from streaming responses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
    pub cache_write_input_tokens: Option<u64>,
}

impl RequestUsage {
    pub fn merge(&mut self, update: Self) {
        merge_max(&mut self.input_tokens, update.input_tokens);
        merge_max(&mut self.output_tokens, update.output_tokens);
        merge_max(
            &mut self.cache_read_input_tokens,
            update.cache_read_input_tokens,
        );
        merge_max(
            &mut self.cache_write_input_tokens,
            update.cache_write_input_tokens,
        );
    }

    pub fn has_usage(&self) -> bool {
        self.input_tokens.is_some() || self.output_tokens.is_some()
    }
}

fn merge_max(target: &mut Option<u64>, value: Option<u64>) {
    if let Some(value) = value {
        *target = Some(target.map_or(value, |current| current.max(value)));
    }
}

/// Session-persisted totals. `cache_read_total_input_tokens` is the weighted
/// denominator for the displayed cache hit ratio and only includes requests
/// where the provider supplied cache details.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub latest_input_tokens: Option<u64>,
    #[serde(default)]
    pub requests: u64,
    #[serde(default)]
    pub requests_with_usage: u64,
    #[serde(default)]
    pub input_reports: u64,
    #[serde(default)]
    pub output_reports: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
    #[serde(default)]
    pub cache_read_total_input_tokens: u64,
    #[serde(default)]
    pub cache_write_input_tokens: u64,
    #[serde(default)]
    pub requests_with_cache_usage: u64,
}

impl TokenUsage {
    pub fn record_request(&mut self, request: &RequestUsage) {
        self.requests = self.requests.saturating_add(1);
        if request.has_usage() {
            self.requests_with_usage = self.requests_with_usage.saturating_add(1);
        }
        if let Some(input) = request.input_tokens {
            self.input_reports = self.input_reports.saturating_add(1);
            self.input_tokens = self.input_tokens.saturating_add(input);
            self.latest_input_tokens = Some(input);
        }
        if let Some(output) = request.output_tokens {
            self.output_reports = self.output_reports.saturating_add(1);
            self.output_tokens = self.output_tokens.saturating_add(output);
        }
        if let Some(cached) = request.cache_read_input_tokens {
            self.cache_read_input_tokens = self.cache_read_input_tokens.saturating_add(cached);
            self.cache_read_total_input_tokens = self
                .cache_read_total_input_tokens
                .saturating_add(request.input_tokens.unwrap_or_default());
            self.requests_with_cache_usage = self.requests_with_cache_usage.saturating_add(1);
        }
        if let Some(cached) = request.cache_write_input_tokens {
            self.cache_write_input_tokens = self.cache_write_input_tokens.saturating_add(cached);
            if request.cache_read_input_tokens.is_none() {
                self.requests_with_cache_usage = self.requests_with_cache_usage.saturating_add(1);
            }
        }
    }

    pub fn add_assign(&mut self, other: &Self) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        if other.latest_input_tokens.is_some() {
            self.latest_input_tokens = other.latest_input_tokens;
        }
        self.requests = self.requests.saturating_add(other.requests);
        self.requests_with_usage = self
            .requests_with_usage
            .saturating_add(other.requests_with_usage);
        self.input_reports = self.input_reports.saturating_add(other.input_reports);
        self.output_reports = self.output_reports.saturating_add(other.output_reports);
        self.cache_read_input_tokens = self
            .cache_read_input_tokens
            .saturating_add(other.cache_read_input_tokens);
        self.cache_read_total_input_tokens = self
            .cache_read_total_input_tokens
            .saturating_add(other.cache_read_total_input_tokens);
        self.cache_write_input_tokens = self
            .cache_write_input_tokens
            .saturating_add(other.cache_write_input_tokens);
        self.requests_with_cache_usage = self
            .requests_with_cache_usage
            .saturating_add(other.requests_with_cache_usage);
    }

    pub fn cache_hit_percent(&self) -> Option<u64> {
        (self.cache_read_total_input_tokens > 0).then(|| {
            self.cache_read_input_tokens
                .saturating_mul(100)
                .checked_div(self.cache_read_total_input_tokens)
                .unwrap_or_default()
                .min(100)
        })
    }
}
