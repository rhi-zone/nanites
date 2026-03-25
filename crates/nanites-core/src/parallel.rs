use std::future::Future;

/// Execute a function in parallel across multiple inputs.
/// Returns all results in order, or the first error encountered.
pub async fn all<I, O, E, F, Fut>(f: F, inputs: Vec<I>) -> Result<Vec<O>, E>
where
    F: Fn(I) -> Fut,
    Fut: Future<Output = Result<O, E>>,
{
    futures::future::try_join_all(inputs.into_iter().map(f)).await
}

/// Execute two functions in parallel, each with its own input.
pub async fn join<IA, IB, OA, OB, EA, EB, FA, FB, FutA, FutB>(
    fa: FA,
    input_a: IA,
    fb: FB,
    input_b: IB,
) -> (Result<OA, EA>, Result<OB, EB>)
where
    FA: FnOnce(IA) -> FutA,
    FB: FnOnce(IB) -> FutB,
    FutA: Future<Output = Result<OA, EA>>,
    FutB: Future<Output = Result<OB, EB>>,
{
    futures::future::join(fa(input_a), fb(input_b)).await
}
