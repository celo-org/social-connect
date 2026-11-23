//! Quota state and discrepancy warnings over the signers' successful answers.
//!
//! Pure functions: the caller counts the returned warnings.

use crate::errors::{
    INCONSISTENT_SIGNER_QUERY_MEASUREMENTS, INCONSISTENT_SIGNER_QUOTA_MEASUREMENTS,
    SIGNER_RESPONSE_DISCREPANCIES,
};

/// A total quota spread or standard deviation above this is a discrepancy.
pub const MAX_TOTAL_QUOTA_DISCREPANCY_THRESHOLD: u64 = 5;
/// A performed query count spread of this much or more is a discrepancy.
pub const MAX_QUERY_COUNT_DISCREPANCY_THRESHOLD: u64 = 5;

/// The quota fields of one successful signer answer.
#[derive(Debug, Clone, PartialEq)]
pub struct SignerQuota {
    pub version: String,
    pub performed_query_count: u64,
    pub total_quota: u64,
    pub warnings: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Warning {
    /// The metric label.
    pub code: &'static str,
    /// The string that goes into the answer.
    pub text: String,
}

fn warning(code: &'static str) -> Warning {
    Warning {
        code,
        text: code.to_owned(),
    }
}

fn spread(values: impl Iterator<Item = u64> + Clone) -> u64 {
    let max = values.clone().max().unwrap_or(0);
    let min = values.min().unwrap_or(0);
    max - min
}

/// `logPnpSignerResponseDiscrepancies`, over successes in arrival order.
///
/// TS compares the `JSON.stringify` output of the four fields, so a missing `warnings` differs
/// from an empty one; `Option` equality does the same.
pub fn discrepancy_warnings(responses: &[SignerQuota]) -> Vec<Warning> {
    let Some(first) = responses.first() else {
        return Vec::new();
    };
    let mut warnings = Vec::new();
    if responses.iter().skip(1).any(|response| response != first) {
        warnings.push(warning(SIGNER_RESPONSE_DISCREPANCIES));
    }
    if spread(responses.iter().map(|r| r.total_quota)) >= MAX_TOTAL_QUOTA_DISCREPANCY_THRESHOLD {
        warnings.push(warning(INCONSISTENT_SIGNER_QUOTA_MEASUREMENTS));
    }
    if spread(responses.iter().map(|r| r.performed_query_count))
        >= MAX_QUERY_COUNT_DISCREPANCY_THRESHOLD
    {
        warnings.push(warning(INCONSISTENT_SIGNER_QUERY_MEASUREMENTS));
    }
    warnings
}

#[derive(Debug, PartialEq)]
pub struct QuotaState {
    pub performed_query_count: u64,
    pub total_quota: u64,
    pub warning: Option<Warning>,
}

/// `findCombinerQuotaState`. `None`: fewer than `threshold` responses, or the standard deviation
/// of the total quota above 5.
///
/// The answer is the response at index `threshold - 1` after a stable sort by remaining quota,
/// most first, so arrival order decides between ties. The warning is counted as `WARN_06`; TS
/// counts it as `WARN_18`.
pub fn quota_state(threshold: usize, responses: &[SignerQuota]) -> Option<QuotaState> {
    if threshold == 0 || responses.len() < threshold {
        return None;
    }
    let mut sorted: Vec<&SignerQuota> = responses.iter().collect();
    // Performed can exceed total, so compute the remaining quota as a signed number.
    sorted.sort_by_key(|r| {
        std::cmp::Reverse(i128::from(r.total_quota) - i128::from(r.performed_query_count))
    });

    let count = sorted.len() as f64;
    let average = sorted.iter().map(|r| r.total_quota as f64).sum::<f64>() / count;
    let variance = sorted
        .iter()
        .map(|r| (r.total_quota as f64 - average).powi(2))
        .sum::<f64>()
        / count;
    let deviation = variance.sqrt();
    if deviation > MAX_TOTAL_QUOTA_DISCREPANCY_THRESHOLD as f64 {
        return None;
    }
    let warning = (deviation > 0.0).then(|| Warning {
        code: INCONSISTENT_SIGNER_QUOTA_MEASUREMENTS,
        text: format!(
            "{INCONSISTENT_SIGNER_QUOTA_MEASUREMENTS}, using threshold signer as best guess"
        ),
    });
    let chosen = sorted[threshold - 1];
    Some(QuotaState {
        performed_query_count: chosen.performed_query_count,
        total_quota: chosen.total_quota,
        warning,
    })
}

/// The most frequent status; the lower status wins a tie; `None` for no statuses.
pub fn majority_status(statuses: &[u16]) -> Option<u16> {
    let mut counts: std::collections::BTreeMap<u16, usize> = std::collections::BTreeMap::new();
    for status in statuses {
        *counts.entry(*status).or_default() += 1;
    }
    // BTreeMap iterates from the lowest status; max_by_key keeps the last maximum, so compare
    // with the status reversed to keep the lowest.
    counts
        .into_iter()
        .max_by_key(|(status, count)| (*count, std::cmp::Reverse(*status)))
        .map(|(status, _)| status)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSION: &str = "1.0.0";
    const TOTAL_QUOTA: u64 = 10;
    const PERFORMED: u64 = 5;
    const THRESHOLD: usize = 3;

    fn signer(performed_query_count: u64, total_quota: u64) -> SignerQuota {
        SignerQuota {
            version: VERSION.to_owned(),
            performed_query_count,
            total_quota,
            warnings: None,
        }
    }

    fn state(responses: &[(u64, u64)]) -> Option<QuotaState> {
        let responses: Vec<_> = responses.iter().map(|&(p, t)| signer(p, t)).collect();
        quota_state(THRESHOLD, &responses)
    }

    fn spread_warning() -> Option<Warning> {
        Some(Warning {
            code: INCONSISTENT_SIGNER_QUOTA_MEASUREMENTS,
            text: format!(
                "{INCONSISTENT_SIGNER_QUOTA_MEASUREMENTS}, using threshold signer as best guess"
            ),
        })
    }

    // pnp-threshold-state.test.ts: varyingQueryCount
    #[test]
    fn performed_query_count_follows_the_threshold_signer() {
        let t = TOTAL_QUOTA;
        let cases: [([u64; 4], u64); 5] = [
            ([0, 0, 0, 0], 0),
            ([0, 0, 0, 1], 0),
            ([1, 1, 0, 1], 1),
            ([0, 1, 1, 1], 1),
            ([1, 4, 9, 11], 9),
        ];
        for (performed, expected) in cases {
            let responses: Vec<_> = performed.iter().map(|&p| (p, t)).collect();
            assert_eq!(
                state(&responses),
                Some(QuotaState {
                    performed_query_count: expected,
                    total_quota: t,
                    warning: None
                }),
                "{performed:?}"
            );
        }
    }

    // pnp-threshold-state.test.ts: varyingTotalQuota
    #[test]
    fn total_quota_follows_the_threshold_signer() {
        let cases: [([u64; 4], u64, bool); 3] = [
            ([10, 10, 10, 10], 10, false),
            ([7, 8, 9, 10], 8, true),
            ([8, 9, 10, 7], 8, true),
        ];
        for (totals, expected, warned) in cases {
            let responses: Vec<_> = totals.iter().map(|&t| (PERFORMED, t)).collect();
            assert_eq!(
                state(&responses),
                Some(QuotaState {
                    performed_query_count: PERFORMED,
                    total_quota: expected,
                    warning: warned.then(|| spread_warning().unwrap()),
                }),
                "{totals:?}"
            );
        }
    }

    // pnp-threshold-state.test.ts: varyingQuotaAndQuery
    #[test]
    fn both_numbers_come_from_the_same_signer() {
        let cases = [
            ([(1, 10), (2, 9), (3, 8), (4, 7)], 3, 8),
            ([(1, 7), (2, 8), (5, 9), (6, 10)], 5, 9),
        ];
        for (responses, performed, total) in cases {
            assert_eq!(
                state(&responses),
                Some(QuotaState {
                    performed_query_count: performed,
                    total_quota: total,
                    warning: spread_warning(),
                }),
                "{responses:?}"
            );
        }
    }

    // pnp-threshold-state.test.ts: 'should throw an error if the total quota varies too much'
    #[test]
    fn a_wide_total_quota_deviation_gives_no_state() {
        let responses = [
            (PERFORMED, 1),
            (PERFORMED, 9),
            (PERFORMED, 15),
            (PERFORMED, 14),
        ];
        assert_eq!(state(&responses), None);
    }

    #[test]
    fn fewer_responses_than_the_threshold_give_no_state() {
        assert_eq!(state(&[(0, 10), (0, 10)]), None);
        assert_eq!(quota_state(0, &[signer(0, 10)]), None);
    }

    #[test]
    fn ties_on_remaining_quota_are_broken_by_arrival_order() {
        let responses = [signer(10, 15), signer(12, 17)];
        assert_eq!(
            quota_state(1, &responses).map(|s| (s.performed_query_count, s.total_quota)),
            Some((10, 15))
        );
        let reversed = [signer(12, 17), signer(10, 15)];
        assert_eq!(
            quota_state(1, &reversed).map(|s| (s.performed_query_count, s.total_quota)),
            Some((12, 17))
        );
    }

    // TS counts this as WARN_18, the label for a query-count disagreement; a total-quota
    // disagreement is WARN_06 (CHANGELOG.md).
    #[test]
    fn total_quota_spread_warning_is_counted_as_warn_06() {
        let responses = [signer(0, 10), signer(0, 11), signer(0, 12)];
        assert_eq!(
            quota_state(1, &responses).unwrap().warning,
            Some(Warning {
                code: INCONSISTENT_SIGNER_QUOTA_MEASUREMENTS,
                text: INCONSISTENT_SIGNER_QUOTA_MEASUREMENTS.to_owned()
                    + ", using threshold signer as best guess",
            })
        );
    }

    #[test]
    fn performed_above_total_sorts_last() {
        let responses = [signer(20, 10), signer(0, 10), signer(5, 10)];
        assert_eq!(
            quota_state(3, &responses).map(|s| s.performed_query_count),
            Some(20)
        );
    }

    fn response(
        performed_query_count: u64,
        total_quota: u64,
        version: &str,
        warnings: &[&str],
    ) -> SignerQuota {
        SignerQuota {
            version: version.to_owned(),
            performed_query_count,
            total_quota,
            warnings: Some(warnings.iter().map(|w| (*w).to_owned()).collect()),
        }
    }

    // pnp-response-logger.test.ts: testCases. The TS table asserts logs only. Where it lists just
    // the error log, the warnings below also hold WARN_17, because the responses differ.
    #[test]
    fn discrepancy_warnings_follow_the_ts_cases() {
        let w = ["warning"];
        let r = |p, t| response(p, t, VERSION, &w);
        let codes = |responses: &[SignerQuota]| -> Vec<&'static str> {
            discrepancy_warnings(responses)
                .iter()
                .map(|x| x.code)
                .collect()
        };
        let none: Vec<&str> = vec![];

        // no responses
        assert_eq!(codes(&[]), none);
        // all the same
        assert_eq!(codes(&[r(5, 10), r(5, 10), r(5, 10)]), none);
        // version differs
        assert_eq!(
            codes(&[response(5, 10, "differentVersion", &w), r(5, 10), r(5, 10)]),
            vec![SIGNER_RESPONSE_DISCREPANCIES]
        );
        // performedQueryCount differs
        assert_eq!(
            codes(&[r(1, 10), r(5, 10), r(5, 10)]),
            vec![SIGNER_RESPONSE_DISCREPANCIES]
        );
        // large performedQueryCount discrepancy
        assert_eq!(
            codes(&[r(10, 10), r(5, 10), r(5, 10)]),
            vec![
                SIGNER_RESPONSE_DISCREPANCIES,
                INCONSISTENT_SIGNER_QUERY_MEASUREMENTS
            ]
        );
        // totalQuota differs
        assert_eq!(
            codes(&[r(5, 10), r(5, 10), r(5, 1)]),
            vec![
                SIGNER_RESPONSE_DISCREPANCIES,
                INCONSISTENT_SIGNER_QUOTA_MEASUREMENTS
            ]
        );
        // large totalQuota discrepancy
        assert_eq!(
            codes(&[r(5, 10), r(5, 10), r(5, 15)]),
            vec![
                SIGNER_RESPONSE_DISCREPANCIES,
                INCONSISTENT_SIGNER_QUOTA_MEASUREMENTS
            ]
        );
        // warnings differ
        assert_eq!(
            codes(&[
                r(5, 10),
                r(5, 10),
                response(5, 10, VERSION, &["differentWarning"])
            ]),
            vec![SIGNER_RESPONSE_DISCREPANCIES]
        );
    }

    #[test]
    fn discrepancy_warning_text_is_the_code() {
        let warnings = discrepancy_warnings(&[signer(0, 10), signer(1, 10)]);
        assert_eq!(
            warnings,
            vec![Warning {
                code: SIGNER_RESPONSE_DISCREPANCIES,
                text: SIGNER_RESPONSE_DISCREPANCIES.to_owned()
            }]
        );
    }

    #[test]
    fn missing_and_empty_warnings_differ() {
        let missing = signer(0, 10);
        let empty = SignerQuota {
            warnings: Some(vec![]),
            ..signer(0, 10)
        };
        let codes: Vec<_> = discrepancy_warnings(&[missing, empty])
            .iter()
            .map(|w| w.code)
            .collect();
        assert_eq!(codes, vec![SIGNER_RESPONSE_DISCREPANCIES]);
    }

    #[test]
    fn majority_status_prefers_the_lower_status_on_a_tie() {
        assert_eq!(majority_status(&[500, 403]), Some(403));
        assert_eq!(majority_status(&[500, 500, 403]), Some(500));
        assert_eq!(majority_status(&[]), None);
    }
}
