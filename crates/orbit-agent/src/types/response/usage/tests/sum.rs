#![allow(missing_docs)]

use serde_json::json;

use super::super::*;

#[test]
fn gemini_cli_model_token_blocks_are_summed_once_per_model() {
    let documents = vec![json!({
        "stats": {
            "models": {
                "gemini-3.1-pro": {
                    "tokens": {
                        "input": 10,
                        "cached": 2,
                        "candidates": 4,
                        "total": 999,
                        "thoughts": 70,
                        "tool": 30
                    },
                    "roles": {
                        "user": {
                            "tokens": {
                                "input": 10,
                                "cached": 2
                            }
                        },
                        "model": {
                            "tokens": {
                                "candidates": 4
                            }
                        }
                    }
                },
                "gemini-2.5-flash": {
                    "tokens": {
                        "prompt": 20,
                        "cached": "3",
                        "output": "5",
                        "total": 28
                    },
                    "roles": {
                        "user": {
                            "tokens": {
                                "prompt": 20
                            }
                        },
                        "model": {
                            "tokens": {
                                "output": 5
                            }
                        }
                    }
                }
            }
        }
    })];

    assert_eq!(
        sum_usage(&documents),
        TokenUsage {
            input: 30,
            cache_read: 5,
            cache_create: 0,
            output: 9,
        }
    );
}

#[test]
fn gemini_cli_role_tokens_are_counted_when_model_tokens_are_absent() {
    let documents = vec![json!({
        "stats": {
            "models": {
                "gemini-3.1-pro": {
                    "roles": {
                        "user": {
                            "tokens": {
                                "input": 7,
                                "cached": 1
                            }
                        },
                        "model": {
                            "tokens": {
                                "candidates": 3
                            }
                        }
                    }
                }
            }
        }
    })];

    assert_eq!(
        sum_usage(&documents),
        TokenUsage {
            input: 7,
            cache_read: 1,
            cache_create: 0,
            output: 3,
        }
    );
}

#[test]
fn gemini_cli_total_thoughts_and_tool_are_not_folded_into_usage() {
    let documents = vec![json!({
        "stats": {
            "models": {
                "gemini-3.1-pro": {
                    "tokens": {
                        "total": 999,
                        "thoughts": 70,
                        "tool": 30
                    }
                }
            }
        }
    })];

    // TokenUsage has no thoughts/tool fields yet, so these Gemini-only
    // counts are intentionally ignored rather than mixed into I/O totals.
    assert_eq!(sum_usage(&documents), TokenUsage::default());
}
