use std::collections::{BTreeMap, BTreeSet};

use super::model::DiscoveredEdge;

pub(super) fn topological_order<I>(edges: &[DiscoveredEdge], packages: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let package_set = packages.into_iter().collect::<BTreeSet<_>>();
    let mut outgoing = package_set
        .iter()
        .map(|package| (package.clone(), BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    let mut incoming = package_set
        .iter()
        .map(|package| (package.clone(), 0usize))
        .collect::<BTreeMap<_, _>>();
    for edge in edges.iter().filter(|edge| !edge.external) {
        if package_set.contains(&edge.source)
            && package_set.contains(&edge.destination)
            && outgoing
                .get_mut(&edge.source)
                .is_some_and(|destinations| destinations.insert(edge.destination.clone()))
            && let Some(count) = incoming.get_mut(&edge.destination)
        {
            *count += 1;
        }
    }
    let mut ready = incoming
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(package, _)| package.clone())
        .collect::<BTreeSet<_>>();
    let mut order = Vec::with_capacity(package_set.len());
    while let Some(package) = ready.pop_first() {
        order.push(package.clone());
        if let Some(destinations) = outgoing.get(&package) {
            for destination in destinations {
                let count = incoming
                    .get_mut(destination)
                    .expect("outgoing package has an incoming count");
                *count -= 1;
                if *count == 0 {
                    ready.insert(destination.clone());
                }
            }
        }
    }
    order
}

pub(super) fn cycle_edges(edges: &[DiscoveredEdge]) -> Vec<(String, String)> {
    let nodes = edges
        .iter()
        .filter(|edge| !edge.external)
        .flat_map(|edge| [edge.source.clone(), edge.destination.clone()])
        .collect::<BTreeSet<_>>();
    let order = topological_order(edges, nodes.iter().cloned());
    let residual = nodes
        .difference(&order.iter().cloned().collect::<BTreeSet<_>>())
        .cloned()
        .collect::<BTreeSet<_>>();
    edges
        .iter()
        .filter(|edge| {
            !edge.external
                && residual.contains(&edge.source)
                && residual.contains(&edge.destination)
        })
        .map(|edge| (edge.source.clone(), edge.destination.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
