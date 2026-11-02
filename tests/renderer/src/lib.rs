// The production modules contain platform entry points unused by this host
// harness. GPU tests record through the production pipelines and frame graph.
#![cfg(test)]
#![allow(dead_code)]

mod ohos;
