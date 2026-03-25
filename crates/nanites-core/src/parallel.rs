use crate::turn::{Turn, TurnError};

/// Execute multiple turns in parallel on their respective inputs.
/// Returns all results in order, or the first error encountered.
pub async fn all<I, O, T>(turn: &T, inputs: Vec<I>) -> Result<Vec<O>, TurnError>
where
    I: Send + 'static,
    O: Send + 'static,
    T: Turn<I, O>,
{
    let mut handles = Vec::with_capacity(inputs.len());
    for input in inputs {
        handles.push(turn.call(input));
    }
    let mut results = Vec::with_capacity(handles.len());
    for handle in handles {
        results.push(handle.await?);
    }
    Ok(results)
}

/// Execute multiple different turns in parallel, each with its own input.
/// Uses `tokio::join!` semantics — all futures are polled concurrently.
pub async fn join<A, B, IA, IB, OA, OB>(
    turn_a: &A,
    input_a: IA,
    turn_b: &B,
    input_b: IB,
) -> Result<(OA, OB), TurnError>
where
    A: Turn<IA, OA>,
    B: Turn<IB, OB>,
    IA: Send + 'static,
    IB: Send + 'static,
    OA: Send + 'static,
    OB: Send + 'static,
{
    let (a, b) = tokio::join!(turn_a.call(input_a), turn_b.call(input_b));
    Ok((a?, b?))
}
