//! Convert attested DEX register-word arguments to Java descriptor operands.
use super::*;

pub(super) fn operands(method: &JavaMethod, args: &[ValueId]) -> Result<Vec<ValueId>, Unsupported> {
    let receiver = usize::from(!method.is_static);
    let logical = receiver + method.parameters.len();
    // Hand-built managed IR may already carry one value per parameter.
    if args.len() == logical {
        return Ok(args.to_vec());
    }
    let words = receiver
        + method
            .parameters
            .iter()
            .map(|t| {
                if matches!(t, JavaType::Long | JavaType::Double) {
                    2
                } else {
                    1
                }
            })
            .sum::<usize>();
    if args.len() != words {
        return Err(Unsupported::Metadata("invoke argument count".into()));
    }
    let mut result = args[..receiver].to_vec();
    let mut word = receiver;
    for ty in &method.parameters {
        let value = args[word];
        result.push(value);
        if matches!(ty, JavaType::Long | JavaType::Double) {
            // The lifter attests both physical words as the same SSA value.
            // Never silently discard a distinct high-word definition.
            if args[word + 1] != value {
                return Err(Unsupported::Metadata(
                    "invoke wide word identity mismatch".into(),
                ));
            }
            word += 2;
        } else {
            word += 1;
        }
    }
    Ok(result)
}
