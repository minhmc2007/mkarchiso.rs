use std::collections::{HashMap, HashSet};

use anyhow::Result;
use petgraph::graph::{Graph, NodeIndex};
use petgraph::visit::IntoNodeIdentifiers;

use super::alpm::DependencyProvider;

pub fn build_dependency_graph(
    packages: &HashSet<String>,
    provider: &dyn DependencyProvider,
) -> Result<Graph<String, ()>> {
    let mut graph = Graph::<String, ()>::new();
    let mut index_map: HashMap<String, NodeIndex> = HashMap::new();

    for pkg in packages {
        let idx = graph.add_node(pkg.clone());
        index_map.insert(pkg.clone(), idx);
    }

    for pkg in packages {
        let deps = provider.get_deps(pkg)?;
        let from = index_map[pkg];
        for dep in deps {
            if let Some(&to) = index_map.get(&dep) {
                graph.add_edge(from, to, ());
            }
        }
    }

    Ok(graph)
}

pub fn partition_into_groups(graph: &Graph<String, ()>) -> Vec<HashSet<String>> {
    let mut visited = HashSet::new();
    let mut groups = Vec::new();

    for node_id in graph.node_identifiers() {
        if visited.contains(&node_id) {
            continue;
        }

        let mut component = HashSet::new();
        let mut stack = vec![node_id];

        while let Some(current) = stack.pop() {
            if !visited.insert(current) {
                continue;
            }
            component.insert(graph[current].clone());

            for neighbor in graph.neighbors_undirected(current) {
                if !visited.contains(&neighbor) {
                    stack.push(neighbor);
                }
            }
        }

        groups.push(component);
    }

    groups
}

pub fn partition_packages(
    packages: &HashSet<String>,
    provider: &dyn DependencyProvider,
) -> Result<Vec<HashSet<String>>> {
    let graph = build_dependency_graph(packages, provider)?;
    Ok(partition_into_groups(&graph))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::alpm::MockDependencyProvider;

    fn set(pks: &[&str]) -> HashSet<String> {
        pks.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn single_isolated_package() {
        let packages = set(&["base"]);
        let provider = MockDependencyProvider::new();

        let groups = partition_packages(&packages, &provider).unwrap();
        assert_eq!(groups.len(), 1);
        assert!(groups[0].contains("base"));
    }

    #[test]
    fn two_independent_packages() {
        let packages = set(&["base", "linux"]);
        let provider = MockDependencyProvider::new();

        let groups = partition_packages(&packages, &provider).unwrap();
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn linear_chain() {
        let mut provider = MockDependencyProvider::new();
        provider.insert("a", &["b"]);
        provider.insert("b", &["c"]);

        let packages = set(&["a", "b", "c"]);
        let groups = partition_packages(&packages, &provider).unwrap();

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 3);
    }

    #[test]
    fn external_dep_ignored() {
        let mut provider = MockDependencyProvider::new();
        provider.insert("a", &["external-pkg"]);

        let packages = set(&["a"]);
        let groups = partition_packages(&packages, &provider).unwrap();

        assert_eq!(groups.len(), 1);
        assert!(groups[0].contains("a"));
    }

    #[test]
    fn two_connected_components() {
        let mut provider = MockDependencyProvider::new();
        provider.insert("a", &["b"]);
        provider.insert("c", &["d"]);

        let packages = set(&["a", "b", "c", "d"]);
        let groups = partition_packages(&packages, &provider).unwrap();

        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn build_graph_edge_direction() {
        let mut provider = MockDependencyProvider::new();
        provider.insert("app", &["lib"]);

        let packages = set(&["app", "lib"]);
        let graph = build_dependency_graph(&packages, &provider).unwrap();

        assert_eq!(graph.edge_count(), 1);

        let app_idx = graph
            .node_indices()
            .find(|i| graph[*i] == "app")
            .unwrap();
        let lib_idx = graph
            .node_indices()
            .find(|i| graph[*i] == "lib")
            .unwrap();

        assert!(graph.contains_edge(app_idx, lib_idx));
        assert!(!graph.contains_edge(lib_idx, app_idx));
    }
}
