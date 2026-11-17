//! AWS security groups from Terraform and CloudFormation, and (opt-in) from
//! a live account through the `aws` CLI, plus the X-Ray service map.
//!
//! A security group belongs to the service it is attached to (ECS service,
//! Lambda function, RDS instance or cluster, ElastiCache), or else to the
//! name in its `app`/`service`/`component`/`Name` tag, its group name or its
//! Terraform label. A rule that lets group A reach group B becomes a candidate
//! call from A's service to B's service or database. CIDR rules add nothing.
//!
//! Names are matched loosely in the merge (`orders-py-prod-sg` finds
//! `orders-py`), but only to components that exist; the rest is unresolved.

use std::collections::{BTreeMap, BTreeSet};

use regex::Regex;
use serde_json::{Value, json};

use crate::raw::RawRecord;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Owner {
    Service(String),
    Database(String),
}

#[derive(Default)]
struct Model {
    /// Group key -> name hint (tag, group name or label).
    groups: BTreeMap<String, String>,
    /// Group key -> its own name, for the firewall-rule node.
    names: BTreeMap<String, String>,
    /// Group keys that are defined here, not looked up with `data`.
    defined: BTreeSet<String>,
    /// Group key -> the resource it is attached to.
    owners: BTreeMap<String, Owner>,
    /// Allowed traffic, from group key to group key.
    rules: BTreeSet<(String, String)>,
    /// Task definition key -> container images, as written.
    task_images: BTreeMap<String, Vec<String>>,
    /// Service name -> the task definition it runs.
    service_tasks: Vec<(String, String)>,
    /// Service or function name -> its container images, as written.
    images: BTreeMap<String, Vec<String>>,
    /// Terraform ECR label -> repository name.
    ecr_names: BTreeMap<String, String>,
    /// Database resource key (`aws_db_instance.label` or a template id) -> identifier.
    db_ids: BTreeMap<String, String>,
    /// Task definition key -> the text that configures its containers.
    task_env: BTreeMap<String, String>,
    /// Lambda function name -> the text of its settings.
    fn_env: BTreeMap<String, String>,
    /// Queue key (`aws_sqs_queue.label` or a template id) -> queue name.
    queues: BTreeMap<String, String>,
    /// Topic key (`aws_sns_topic.label` or a template id) -> topic name.
    topics: BTreeMap<String, String>,
    /// Function key -> function name.
    fn_keys: BTreeMap<String, String>,
    /// A function that reads a queue: (function key or name, queue key).
    reads: Vec<(String, String)>,
    /// A topic delivered to a queue or a function: (topic key, protocol, endpoint key or name).
    subscriptions: Vec<(String, String, String)>,
}

/// Records from the Terraform files of one project (path, text), with local
/// modules expanded: each `module` call with a `./` or `../` source becomes
/// its own copy of the module's files, with literal inputs and variable
/// defaults filled in. Registry and Git modules cannot be read and stay as
/// they are; so do inputs that are not plain strings.
#[must_use]
pub fn terraform_records_from_files(
    project_path: &str,
    files: &[(String, String)],
) -> Vec<RawRecord> {
    terraform_records(project_path, &expand_modules(files))
}

const MODULE_DEPTH: usize = 4;

fn dir_of(path: &str) -> String {
    path.rsplit_once('/')
        .map_or_else(String::new, |(dir, _)| dir.to_owned())
}

/// Resolve `source` (starting with `./` or `../`) against `dir`.
fn join_dir(dir: &str, source: &str) -> Option<String> {
    let mut parts: Vec<&str> = dir.split('/').filter(|part| !part.is_empty()).collect();
    for part in source.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

struct ModuleCall {
    label: String,
    target: String,
    inputs: BTreeMap<String, String>,
}

fn variable_defaults(text: &str) -> BTreeMap<String, String> {
    let header = Regex::new(r#"(?m)^\s*variable\s+"([^"]+)"\s*\{"#).expect("regex");
    header
        .captures_iter(text)
        .filter_map(|caps| {
            let body = braced(text, caps.get(0)?.end() - 1)?;
            Some((caps[1].to_owned(), tf_string(&top_level(body), "default")?))
        })
        .collect()
}

/// Plain-string `locals`, with references to earlier locals filled in.
/// A local that still needs a function or another unknown is left out.
fn locals_of(text: &str) -> BTreeMap<String, String> {
    let header = Regex::new(r"(?m)^\s*locals\s*\{").expect("regex");
    let entry = Regex::new(r#"(?m)^\s*([A-Za-z0-9_]+)\s*=\s*"([^"]*)""#).expect("regex");
    let mut raw: BTreeMap<String, String> = BTreeMap::new();
    for found in header.find_iter(text) {
        if let Some(body) = braced(text, found.end() - 1) {
            for caps in entry.captures_iter(&top_level(body)) {
                raw.insert(caps[1].to_owned(), caps[2].to_owned());
            }
        }
    }
    let mut known: BTreeMap<String, String> = BTreeMap::new();
    for _ in 0..4 {
        for (name, value) in &raw {
            let value = substitute_refs(value, "local", &known);
            if !value.contains("${") {
                known.insert(name.clone(), value);
            }
        }
    }
    known
}

fn substitute_vars(text: &str, values: &BTreeMap<String, String>) -> String {
    substitute_refs(text, "var", values)
}

/// Replace `${prefix.name}` and a bare `prefix.name` with the value.
fn substitute_refs(text: &str, prefix: &str, values: &BTreeMap<String, String>) -> String {
    let mut out = text.to_owned();
    for (name, value) in values {
        out = out.replace(&format!("${{{prefix}.{name}}}"), value);
        let bare = Regex::new(&format!(r"\b{prefix}\.{}\b", regex::escape(name))).expect("regex");
        out = bare.replace_all(&out, format!("\"{value}\"")).into_owned();
    }
    out
}

fn module_calls(dir: &str, text: &str, defaults: &BTreeMap<String, String>) -> Vec<ModuleCall> {
    let header = Regex::new(r#"(?m)^\s*module\s+"([^"]+)"\s*\{"#).expect("regex");
    let input = Regex::new(r#"(?m)^\s*([A-Za-z0-9_]+)\s*=\s*"([^"]*)""#).expect("regex");
    header
        .captures_iter(text)
        .filter_map(|caps| {
            let body = braced(text, caps.get(0)?.end() - 1)?;
            let top = top_level(body);
            let source = tf_string(&top, "source")?;
            if !source.starts_with("./") && !source.starts_with("../") {
                return None;
            }
            let inputs = input
                .captures_iter(&top)
                .filter(|pair| &pair[1] != "source" && &pair[1] != "version")
                .map(|pair| {
                    (
                        pair[1].to_owned(),
                        substitute_vars(&pair[2], defaults).replace('"', ""),
                    )
                })
                .collect();
            Some(ModuleCall {
                label: caps[1].to_owned(),
                target: join_dir(dir, &source)?,
                inputs,
            })
        })
        .collect()
}

/// One copy of a module: inputs and defaults filled in, resource labels made
/// unique with `tag`, and the modules it calls expanded the same way.
fn module_instance(
    dirs: &BTreeMap<String, String>,
    call: &ModuleCall,
    tag: &str,
    depth: usize,
    out: &mut String,
) {
    let Some(text) = dirs.get(&call.target) else {
        return;
    };
    if depth >= MODULE_DEPTH {
        return;
    }
    let mut values = variable_defaults(text);
    values.extend(call.inputs.clone());
    let mut text = substitute_vars(text, &values);
    text = substitute_refs(&text, "local", &locals_of(&text));
    let header =
        Regex::new(r#"(?m)^\s*(?:resource|data)\s+"([^"]+)"\s+"([^"]+)"\s*\{"#).expect("regex");
    let labels: Vec<(String, String)> = header
        .captures_iter(&text)
        .map(|caps| (caps[1].to_owned(), caps[2].to_owned()))
        .collect();
    for (rtype, label) in labels {
        let escaped = (regex::escape(&rtype), regex::escape(&label));
        let declaration =
            Regex::new(&format!(r#"("{}"\s+)"{}""#, escaped.0, escaped.1)).expect("regex");
        text = declaration
            .replace_all(&text, format!(r#"${{1}}"{label}__{tag}""#))
            .into_owned();
        let reference = Regex::new(&format!(
            r"\b{}\.{}([^A-Za-z0-9_-]|$)",
            escaped.0, escaped.1
        ))
        .expect("regex");
        text = reference
            .replace_all(&text, format!("{rtype}.{label}__{tag}${{1}}"))
            .into_owned();
    }
    out.push_str(&text);
    out.push('\n');
    let own = variable_defaults(&text);
    for nested in module_calls(&call.target, &text, &own) {
        let nested_tag = format!("{tag}_{}", nested.label);
        module_instance(dirs, &nested, &nested_tag, depth + 1, out);
    }
}

fn expand_modules(files: &[(String, String)]) -> String {
    let mut dirs: BTreeMap<String, String> = BTreeMap::new();
    for (path, text) in files.iter().filter(|(path, _)| path.ends_with(".tf")) {
        let entry = dirs.entry(dir_of(path)).or_default();
        entry.push_str(text);
        entry.push('\n');
    }
    for text in dirs.values_mut() {
        *text = substitute_refs(text, "local", &locals_of(text));
    }
    let mut instances = String::new();
    let mut instantiated: BTreeSet<String> = BTreeSet::new();
    for (dir, text) in &dirs {
        let defaults = variable_defaults(text);
        for call in module_calls(dir, text, &defaults) {
            if dirs.contains_key(&call.target) && &call.target != dir {
                instantiated.insert(call.target.clone());
                module_instance(&dirs, &call, &call.label.clone(), 0, &mut instances);
            }
        }
    }
    let mut out = String::new();
    for (dir, text) in &dirs {
        // A module read through its calls would otherwise be read twice, once
        // with its inputs left unfilled.
        if !instantiated.contains(dir) {
            out.push_str(text);
        }
    }
    out.push_str(&instances);
    out
}

/// Records from all Terraform files of one project, read together so that
/// references across files resolve.
#[must_use]
pub fn terraform_records(project_path: &str, text: &str) -> Vec<RawRecord> {
    if ![
        "aws_security_group",
        "aws_ecs_service",
        "aws_lambda_function",
        "aws_sqs_queue",
        "aws_sns_topic",
    ]
    .iter()
    .any(|needle| text.contains(needle))
    {
        return Vec::new();
    }
    let mut model = Model::default();
    let task_ref = Regex::new(r"aws_ecs_task_definition\.([A-Za-z0-9_-]+)").expect("regex");
    let header =
        Regex::new(r#"(?m)^\s*(resource|data)\s+"([^"]+)"\s+"([^"]+)"\s*\{"#).expect("valid regex");
    for caps in header.captures_iter(text) {
        let whole = caps.get(0).expect("match");
        let Some(body) = braced(text, whole.end() - 1) else {
            continue;
        };
        let kind = &caps[1];
        let rtype = &caps[2];
        let label = &caps[3];
        let top = top_level(body);
        match (kind, rtype) {
            (_, "aws_security_group") => {
                let key = if kind == "data" {
                    format!("data.aws_security_group.{label}")
                } else {
                    format!("aws_security_group.{label}")
                };
                let tags = tf_tags(body);
                let own = tf_string(&top, "name")
                    .or_else(|| tf_string(&top, "name_prefix"))
                    .or_else(|| tags.get("Name").cloned())
                    .unwrap_or_else(|| label.to_owned());
                let hint = tag_hint(&tags).unwrap_or_else(|| own.clone());
                model.groups.insert(key.clone(), hint);
                model.names.insert(key.clone(), own);
                if kind == "resource" {
                    model.defined.insert(key.clone());
                    for block in nested(body, "ingress") {
                        for from in sg_refs(&tf_value(block, "security_groups")) {
                            model.rules.insert((from, key.clone()));
                        }
                    }
                    for block in nested(body, "egress") {
                        for to in sg_refs(&tf_value(block, "security_groups")) {
                            model.rules.insert((key.clone(), to));
                        }
                    }
                }
            }
            ("resource", "aws_security_group_rule") => {
                let group = sg_refs(&tf_value(&top, "security_group_id"));
                let other = sg_refs(&tf_value(&top, "source_security_group_id"));
                let ingress = tf_string(&top, "type").as_deref() == Some("ingress");
                for group in &group {
                    for other in &other {
                        model.rules.insert(if ingress {
                            (other.clone(), group.clone())
                        } else {
                            (group.clone(), other.clone())
                        });
                    }
                }
            }
            (
                "resource",
                "aws_vpc_security_group_ingress_rule" | "aws_vpc_security_group_egress_rule",
            ) => {
                let group = sg_refs(&tf_value(&top, "security_group_id"));
                let other = sg_refs(&tf_value(&top, "referenced_security_group_id"));
                let ingress = rtype.ends_with("ingress_rule");
                for group in &group {
                    for other in &other {
                        model.rules.insert(if ingress {
                            (other.clone(), group.clone())
                        } else {
                            (group.clone(), other.clone())
                        });
                    }
                }
            }
            ("resource", "aws_ecr_repository") => {
                let name = tf_string(&top, "name").unwrap_or_else(|| label.to_owned());
                model.ecr_names.insert(label.to_owned(), name);
            }
            ("resource", "aws_ecs_task_definition") => {
                let images = tf_images(body);
                model.task_images.insert(label.to_owned(), images.clone());
                model.task_env.insert(label.to_owned(), body.to_owned());
                if let Some(family) = tf_string(&top, "family") {
                    model.task_images.insert(family.clone(), images);
                    model.task_env.insert(family, body.to_owned());
                }
            }
            ("resource", "aws_ecs_service") => {
                let name = tf_string(&top, "name");
                if let Some(name) = &name {
                    let reference = tf_value(&top, "task_definition");
                    let task = task_ref
                        .captures(&reference)
                        .map(|caps| caps[1].to_owned())
                        .or_else(|| tf_string(&top, "task_definition"));
                    if let Some(task) = task {
                        model.service_tasks.push((name.clone(), task));
                    }
                }
                let refs: Vec<String> = nested(body, "network_configuration")
                    .iter()
                    .flat_map(|block| sg_refs(&tf_value(block, "security_groups")))
                    .collect();
                attach(&mut model, refs, name.map(Owner::Service));
            }
            ("resource", "aws_sqs_queue") => {
                let name = tf_string(&top, "name").unwrap_or_else(|| label.to_owned());
                model.queues.insert(format!("aws_sqs_queue.{label}"), name);
            }
            ("resource", "aws_sns_topic") => {
                let name = tf_string(&top, "name").unwrap_or_else(|| label.to_owned());
                model.topics.insert(format!("aws_sns_topic.{label}"), name);
            }
            ("resource", "aws_lambda_event_source_mapping") => {
                if let (Some(queue), Some(function)) = (
                    tf_ref(&tf_value(&top, "event_source_arn"), "aws_sqs_queue"),
                    tf_function(&tf_value(&top, "function_name")),
                ) {
                    model.reads.push((function, queue));
                }
            }
            ("resource", "aws_sns_topic_subscription") => {
                let protocol = tf_string(&top, "protocol").unwrap_or_default();
                let endpoint = tf_value(&top, "endpoint");
                let target = match protocol.as_str() {
                    "lambda" => tf_function(&endpoint),
                    "sqs" => tf_ref(&endpoint, "aws_sqs_queue"),
                    _ => None,
                };
                if let (Some(topic), Some(target)) = (
                    tf_ref(&tf_value(&top, "topic_arn"), "aws_sns_topic"),
                    target,
                ) {
                    model.subscriptions.push((topic, protocol, target));
                }
            }
            ("resource", "aws_lambda_function") => {
                let name = tf_string(&top, "function_name");
                if let Some(name) = &name {
                    model
                        .fn_keys
                        .insert(format!("aws_lambda_function.{label}"), name.clone());
                    model.fn_env.insert(name.clone(), body.to_owned());
                    if let Some(uri) = tf_string(&top, "image_uri") {
                        model.images.entry(name.clone()).or_default().push(uri);
                    }
                }
                let refs: Vec<String> = nested(body, "vpc_config")
                    .iter()
                    .flat_map(|block| sg_refs(&tf_value(block, "security_group_ids")))
                    .collect();
                attach(&mut model, refs, name.map(Owner::Service));
            }
            ("resource", "aws_db_instance" | "aws_rds_cluster") => {
                let name =
                    tf_string(&top, "identifier").or_else(|| tf_string(&top, "cluster_identifier"));
                if let Some(name) = &name {
                    model
                        .db_ids
                        .insert(format!("{rtype}.{label}"), name.clone());
                }
                let refs = sg_refs(&tf_value(&top, "vpc_security_group_ids"));
                attach(&mut model, refs, name.map(Owner::Database));
            }
            ("resource", "aws_elasticache_cluster" | "aws_elasticache_replication_group") => {
                let name = tf_string(&top, "cluster_id")
                    .or_else(|| tf_string(&top, "replication_group_id"));
                if let Some(name) = &name {
                    model
                        .db_ids
                        .insert(format!("{rtype}.{label}"), name.clone());
                }
                let refs = sg_refs(&tf_value(&top, "security_group_ids"));
                attach(&mut model, refs, name.map(Owner::Database));
            }
            _ => {}
        }
    }
    emit(project_path, &model)
}

/// Records from a CloudFormation template in YAML, short-form tags included.
#[must_use]
pub fn cloudformation_records(project_path: &str, text: &str) -> Vec<RawRecord> {
    let Ok(value) = yaml_serde::from_str::<Value>(&untag(text)) else {
        return Vec::new();
    };
    let Some(resources) = value.get("Resources").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut model = Model::default();
    for (id, resource) in resources {
        let props = resource.get("Properties").unwrap_or(&Value::Null);
        let field = |key: &str| props.get(key).unwrap_or(&Value::Null);
        match resource.get("Type").and_then(Value::as_str).unwrap_or("") {
            "AWS::EC2::SecurityGroup" => {
                let tags: BTreeMap<String, String> = field("Tags")
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|tag| {
                        Some((
                            tag.get("Key")?.as_str()?.to_owned(),
                            tag.get("Value")?.as_str()?.to_owned(),
                        ))
                    })
                    .collect();
                let own = field("GroupName")
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| tags.get("Name").cloned())
                    .unwrap_or_else(|| id.clone());
                let hint = tag_hint(&tags).unwrap_or_else(|| own.clone());
                model.groups.insert(id.clone(), hint);
                model.names.insert(id.clone(), own);
                model.defined.insert(id.clone());
                for rule in field("SecurityGroupIngress")
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    if let Some(from) = rule.get("SourceSecurityGroupId").and_then(cfn_ref) {
                        model.rules.insert((from, id.clone()));
                    }
                }
                for rule in field("SecurityGroupEgress")
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    if let Some(to) = rule.get("DestinationSecurityGroupId").and_then(cfn_ref) {
                        model.rules.insert((id.clone(), to));
                    }
                }
            }
            "AWS::EC2::SecurityGroupIngress" => {
                if let (Some(group), Some(from)) = (
                    cfn_ref(field("GroupId")),
                    cfn_ref(field("SourceSecurityGroupId")),
                ) {
                    model.rules.insert((from, group));
                }
            }
            "AWS::EC2::SecurityGroupEgress" => {
                if let (Some(group), Some(to)) = (
                    cfn_ref(field("GroupId")),
                    cfn_ref(field("DestinationSecurityGroupId")),
                ) {
                    model.rules.insert((group, to));
                }
            }
            "AWS::ECS::TaskDefinition" => {
                let images: Vec<String> = field("ContainerDefinitions")
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|container| cfn_text(container.get("Image")?))
                    .collect();
                model.task_images.insert(id.clone(), images);
                model
                    .task_env
                    .insert(id.clone(), field("ContainerDefinitions").to_string());
            }
            "AWS::ECS::Service" => {
                if let Some(task) = cfn_ref(field("TaskDefinition")) {
                    let name = field("ServiceName").as_str().unwrap_or(id).to_owned();
                    model.service_tasks.push((name, task));
                }
                let refs = cfn_refs(
                    field("NetworkConfiguration")
                        .get("AwsvpcConfiguration")
                        .and_then(|config| config.get("SecurityGroups")),
                );
                let name = field("ServiceName").as_str().unwrap_or(id).to_owned();
                attach(&mut model, refs, Some(Owner::Service(name)));
            }
            "AWS::SQS::Queue" => {
                let name = field("QueueName").as_str().unwrap_or(id).to_owned();
                model.queues.insert(id.clone(), name);
            }
            "AWS::SNS::Topic" => {
                let name = field("TopicName").as_str().unwrap_or(id).to_owned();
                model.topics.insert(id.clone(), name);
            }
            "AWS::Lambda::EventSourceMapping" => {
                if let (Some(queue), Some(function)) = (
                    cfn_ref(field("EventSourceArn")),
                    cfn_ref(field("FunctionName"))
                        .or_else(|| field("FunctionName").as_str().map(str::to_owned)),
                ) {
                    model.reads.push((function, queue));
                }
            }
            "AWS::SNS::Subscription" => {
                let protocol = field("Protocol").as_str().unwrap_or("").to_owned();
                if let (Some(topic), Some(target), true) = (
                    cfn_ref(field("TopicArn")),
                    cfn_ref(field("Endpoint")),
                    protocol == "sqs" || protocol == "lambda",
                ) {
                    model.subscriptions.push((topic, protocol, target));
                }
            }
            "AWS::Lambda::Function" => {
                model.fn_keys.insert(
                    id.clone(),
                    field("FunctionName").as_str().unwrap_or(id).to_owned(),
                );
                if let Some(uri) = field("Code").get("ImageUri").and_then(cfn_text) {
                    let name = field("FunctionName").as_str().unwrap_or(id).to_owned();
                    model.images.entry(name).or_default().push(uri);
                }
                let name = field("FunctionName").as_str().unwrap_or(id).to_owned();
                model.fn_env.insert(name, field("Environment").to_string());
                let refs = cfn_refs(
                    field("VpcConfig")
                        .get("SecurityGroupIds")
                        .or(field("VpcConfig").get("SecurityGroups")),
                );
                let name = field("FunctionName").as_str().unwrap_or(id).to_owned();
                attach(&mut model, refs, Some(Owner::Service(name)));
            }
            "AWS::RDS::DBInstance" | "AWS::RDS::DBCluster" => {
                let refs = cfn_refs(
                    props
                        .get("VPCSecurityGroups")
                        .or(props.get("VpcSecurityGroupIds")),
                );
                let name = field("DBInstanceIdentifier")
                    .as_str()
                    .or(field("DBClusterIdentifier").as_str())
                    .unwrap_or(id)
                    .to_owned();
                model.db_ids.insert(id.clone(), name.clone());
                attach(&mut model, refs, Some(Owner::Database(name)));
            }
            _ => {}
        }
    }
    emit(project_path, &model)
}

/// Container images named in a task definition body (HCL or JSON).
fn tf_images(body: &str) -> Vec<String> {
    let image = Regex::new(r#"(?m)"?\bimage"?\s*[:=]\s*"([^"]+)""#).expect("regex");
    image
        .captures_iter(body)
        .map(|caps| caps[1].to_owned())
        .collect()
}

/// A CloudFormation string, or the template of `Fn::Sub`.
fn cfn_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Object(map) => match map.get("Fn::Sub")? {
            Value::String(text) => Some(text.clone()),
            Value::Array(parts) => parts.first()?.as_str().map(str::to_owned),
            _ => None,
        },
        _ => None,
    }
}

/// The repository name of an image reference: the last path segment without
/// its tag or digest. Interpolated names are skipped, not guessed.
fn image_repo(raw: &str, ecr: &BTreeMap<String, String>) -> Option<String> {
    let label = Regex::new(r"aws_ecr_repository\.([A-Za-z0-9_-]+)").expect("regex");
    if let Some(caps) = label.captures(raw) {
        let name = ecr
            .get(&caps[1])
            .cloned()
            .unwrap_or_else(|| caps[1].to_owned());
        let name = name.rsplit('/').next().unwrap_or(&name);
        return Some(kebab(name)).filter(|name| !name.is_empty());
    }
    let last = raw.rsplit('/').next()?;
    let name = last.split([':', '@']).next()?;
    if name.contains("${") {
        return None;
    }
    Some(kebab(name)).filter(|name| !name.is_empty())
}

fn image_records(model: &Model) -> Vec<RawRecord> {
    let mut by_workload: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut add = |workload: &str, raws: &[String]| {
        let workload = kebab(workload);
        if workload.is_empty() {
            return;
        }
        let entry = by_workload.entry(workload).or_default();
        for raw in raws {
            if let Some(name) = image_repo(raw, &model.ecr_names)
                && !entry.contains(&name)
            {
                entry.push(name);
            }
        }
    };
    for (service, task) in &model.service_tasks {
        if let Some(raws) = model.task_images.get(task_key(task)) {
            add(service, raws);
        }
    }
    for (name, raws) in &model.images {
        add(name, raws);
    }
    by_workload
        .into_iter()
        .filter(|(_, images)| !images.is_empty())
        .map(|(workload, images)| {
            record(
                "workload_image",
                json!({"workload": workload, "images": images}),
            )
        })
        .collect()
}

fn attach(model: &mut Model, refs: Vec<String>, owner: Option<Owner>) {
    let Some(owner) = owner else {
        return;
    };
    for key in refs {
        model.owners.entry(key).or_insert_with(|| owner.clone());
    }
}

fn owner_of(model: &Model, key: &str) -> Option<Owner> {
    if let Some(owner) = model.owners.get(key) {
        return Some(owner.clone());
    }
    model
        .groups
        .get(key)
        .map(|hint| Owner::Service(hint.clone()))
}

fn emit(project_path: &str, model: &Model) -> Vec<RawRecord> {
    let mut records = Vec::new();
    for key in &model.defined {
        let Some(Owner::Service(owner)) = owner_of(model, key) else {
            continue;
        };
        let name = kebab(model.names.get(key).map_or(key.as_str(), String::as_str));
        let owner = kebab(&owner);
        if name.is_empty() || owner.is_empty() {
            continue;
        }
        records.push(record(
            "resource",
            json!({"resource_kind": "firewall-rule", "name": name, "project_path": project_path, "component": owner, "loose": true}),
        ));
    }
    for (from, to) in &model.rules {
        let (Some(Owner::Service(source)), Some(target)) =
            (owner_of(model, from), owner_of(model, to))
        else {
            continue;
        };
        let source = kebab(&source);
        let (destination, kind) = match target {
            Owner::Service(name) => (kebab(&name), None),
            Owner::Database(name) => (kebab(&name), Some("database")),
        };
        if source.is_empty() || destination.is_empty() || (kind.is_none() && source == destination)
        {
            continue;
        }
        records.push(record(
            "candidate_call",
            json!({"source": source, "destination": destination, "destination_kind": kind, "loose": true}),
        ));
    }
    let seen: BTreeSet<(String, String)> = records
        .iter()
        .filter(|record| record.kind == "candidate_call")
        .filter_map(|record| {
            Some((
                record.payload["source"].as_str()?.to_owned(),
                record.payload["destination"].as_str()?.to_owned(),
            ))
        })
        .collect();
    for (source, destination) in database_calls(model) {
        if !seen.contains(&(source.clone(), destination.clone())) {
            records.push(record(
                "candidate_call",
                json!({"source": source, "destination": destination, "destination_kind": "database", "loose": true}),
            ));
        }
    }
    records.extend(image_records(model));
    records.extend(messaging_records(model));
    records
}

/// `type.label` for the first reference to a resource of this type.
fn tf_ref(value: &str, rtype: &str) -> Option<String> {
    let pattern =
        Regex::new(&format!(r"(?:^|[^A-Za-z0-9_]){rtype}\.([A-Za-z0-9_-]+)")).expect("regex");
    pattern
        .captures(value)
        .map(|caps| format!("{rtype}.{}", &caps[1]))
}

/// A function given by reference (`aws_lambda_function.x.arn`) or by name.
fn tf_function(value: &str) -> Option<String> {
    tf_ref(value, "aws_lambda_function").or_else(|| {
        let name = value.trim().strip_prefix('"')?.split('"').next()?;
        (!name.is_empty() && !name.contains("${")).then(|| name.to_owned())
    })
}

/// Queues and topics from the template: who reads a queue, which queue a
/// topic feeds, and which services and functions name a queue or topic in
/// their settings.
fn messaging_records(model: &Model) -> Vec<RawRecord> {
    let strip = |name: &str| kebab(name.strip_suffix(".fifo").unwrap_or(name));
    // A function is known by its name; a reference to one without a literal
    // name is skipped rather than guessed, and an ARN is reduced to its name.
    let function = |key: &str| -> Option<String> {
        let name = match model.fn_keys.get(key) {
            Some(name) => name.as_str(),
            None if key.starts_with("aws_lambda_function.") => return None,
            None => key.rsplit(':').next().unwrap_or(key),
        };
        let name = kebab(name);
        (!name.is_empty()).then_some(name)
    };
    let message = |source: &str, direction: &str, kind: &str, name: &str| {
        record(
            "messaging",
            json!({"source": source, "direction": direction, "kind": kind, "name": name, "loose": true}),
        )
    };
    let mut records = Vec::new();
    let mut readers: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for (function_key, queue_key) in &model.reads {
        let Some(queue) = model.queues.get(queue_key) else {
            continue;
        };
        let Some(reader) = function(function_key) else {
            continue;
        };
        records.push(message(&reader, "consume", "queue", &strip(queue)));
        readers.entry(queue_key).or_default().insert(reader);
    }
    for (topic_key, protocol, target) in &model.subscriptions {
        let Some(topic) = model.topics.get(topic_key) else {
            continue;
        };
        if protocol == "lambda" {
            if let Some(reader) = function(target) {
                records.push(message(&reader, "consume", "topic", &strip(topic)));
            }
        } else if let Some(queue) = model.queues.get(target) {
            records.push(record(
                "resource_link",
                json!({
                    "from_kind": "queue", "from": strip(queue),
                    "to_kind": "topic", "to": strip(topic),
                }),
            ));
        }
    }
    let reference = Regex::new(
        r#"(aws_sns_topic|aws_sqs_queue)\.([A-Za-z0-9_-]+)|"(?:Ref"\s*:\s*"|Fn::GetAtt"\s*:\s*\[")([A-Za-z0-9]+)""#,
    )
    .expect("regex");
    for (owner, text) in settings(model) {
        let source = kebab(&owner);
        for caps in reference.captures_iter(text) {
            let key = match (caps.get(1), caps.get(2), caps.get(3)) {
                (Some(kind), Some(label), _) => format!("{}.{}", kind.as_str(), label.as_str()),
                (_, _, Some(id)) => id.as_str().to_owned(),
                _ => continue,
            };
            if let Some(topic) = model.topics.get(&key) {
                records.push(message(&source, "publish", "topic", &strip(topic)));
            } else if let (Some(queue), Some(read_by)) =
                (model.queues.get(&key), readers.get(key.as_str()))
            {
                // A queue only counts as a destination when someone reads it,
                // and a function does not publish to the queue it reads.
                if !read_by.contains(&source) {
                    records.push(message(&source, "publish", "queue", &strip(queue)));
                }
            }
        }
    }
    records
}

/// The settings text of every service task and function, by owner.
fn settings(model: &Model) -> Vec<(String, &String)> {
    let mut settings: Vec<(String, &String)> = Vec::new();
    for (service, task) in &model.service_tasks {
        if let Some(text) = model.task_env.get(task_key(task)) {
            settings.push((service.clone(), text));
        }
    }
    for (name, text) in &model.fn_env {
        settings.push((name.clone(), text));
    }
    settings
}

/// Services and functions whose settings name a database: a reference to the
/// database resource, or a host name that starts with its identifier.
fn database_calls(model: &Model) -> BTreeSet<(String, String)> {
    let resource = Regex::new(
        r"(aws_db_instance|aws_rds_cluster|aws_elasticache_cluster|aws_elasticache_replication_group)\.([A-Za-z0-9_-]+)",
    )
    .expect("regex");
    let template =
        Regex::new(r#""(?:Ref"\s*:\s*"|Fn::GetAtt"\s*:\s*\[")([A-Za-z0-9]+)""#).expect("regex");
    let host = Regex::new(r"([a-z0-9][a-z0-9-]*)\.(?:[a-z0-9-]+\.)+(?:rds|cache)\.amazonaws\.com")
        .expect("regex");
    let mut calls = BTreeSet::new();
    for (owner, text) in settings(model) {
        let source = kebab(&owner);
        let mut found: Vec<String> = Vec::new();
        for caps in resource.captures_iter(text) {
            found.extend(
                model
                    .db_ids
                    .get(&format!("{}.{}", &caps[1], &caps[2]))
                    .cloned(),
            );
        }
        for caps in template.captures_iter(text) {
            found.extend(model.db_ids.get(&caps[1]).cloned());
        }
        found.extend(host.captures_iter(text).map(|caps| caps[1].to_owned()));
        for destination in found {
            let destination = kebab(&destination);
            if !source.is_empty() && !destination.is_empty() {
                calls.insert((source.clone(), destination));
            }
        }
    }
    calls
}

/// A task definition reference without its ARN path or revision.
fn task_key(task: &str) -> &str {
    let task = task.rsplit('/').next().unwrap_or(task);
    task.split(':').next().unwrap_or(task)
}

fn record(kind: &str, payload: Value) -> RawRecord {
    RawRecord {
        collector: "k8s_workloads".to_owned(),
        kind: kind.to_owned(),
        payload,
    }
}

fn tag_hint(tags: &BTreeMap<String, String>) -> Option<String> {
    [
        "app",
        "App",
        "service",
        "Service",
        "component",
        "Component",
        "Name",
    ]
    .iter()
    .find_map(|key| tags.get(*key).filter(|value| !value.is_empty()).cloned())
}

/// Lowercase kebab-case; Terraform interpolations are dropped.
fn kebab(text: &str) -> String {
    let interpolation = Regex::new(r"\$\{[^}]*\}").expect("valid regex");
    let text = interpolation.replace_all(text, "-");
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

/// The text between the brace at `open` and its match.
fn braced(text: &str, open: usize) -> Option<&str> {
    let bytes = text.as_bytes();
    if bytes.get(open) != Some(&b'{') {
        return None;
    }
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, &byte) in bytes.iter().enumerate().skip(open) {
        if in_string {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return text.get(open + 1..index);
                }
            }
            _ => {}
        }
    }
    None
}

/// The body with every nested `{...}` block removed. Braces inside quoted
/// strings (`"${var.name}"`) are text, not blocks.
fn top_level(body: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for c in body.chars() {
        if in_string {
            if depth == 0 {
                out.push(c);
            }
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                if depth == 0 {
                    out.push(c);
                }
            }
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// Bodies of the nested blocks called `name`.
fn nested<'a>(body: &'a str, name: &str) -> Vec<&'a str> {
    let pattern = Regex::new(&format!(r"(?m)^\s*{}\s*\{{", regex::escape(name))).expect("regex");
    pattern
        .find_iter(body)
        .filter_map(|found| braced(body, found.end() - 1))
        .collect()
}

fn tf_value(body: &str, key: &str) -> String {
    let pattern = Regex::new(&format!(
        r"(?ms)^\s*{}\s*=\s*(\[.*?\]|[^\n]*)",
        regex::escape(key)
    ))
    .expect("regex");
    pattern
        .captures(body)
        .map(|caps| caps[1].to_owned())
        .unwrap_or_default()
}

fn tf_string(body: &str, key: &str) -> Option<String> {
    let value = tf_value(body, key);
    let value = value.trim();
    let inner = value.strip_prefix('"')?.split('"').next()?;
    (!inner.is_empty()).then(|| inner.to_owned())
}

fn tf_tags(body: &str) -> BTreeMap<String, String> {
    let pattern = Regex::new(r"(?m)^\s*tags\s*=\s*\{").expect("regex");
    let line = Regex::new(r#"(?m)^\s*"?([A-Za-z0-9_:-]+)"?\s*=\s*"([^"]*)""#).expect("regex");
    let mut tags = BTreeMap::new();
    for found in pattern.find_iter(body) {
        if let Some(inner) = braced(body, found.end() - 1) {
            for caps in line.captures_iter(inner) {
                tags.insert(caps[1].to_owned(), caps[2].to_owned());
            }
        }
    }
    tags
}

fn sg_refs(value: &str) -> Vec<String> {
    let pattern =
        Regex::new(r"((?:data\.)?aws_security_group\.[A-Za-z0-9_-]+)").expect("valid regex");
    pattern
        .captures_iter(value)
        .map(|caps| caps[1].to_owned())
        .collect()
}

/// Turn `!Ref X` and `!GetAtt X.GroupId` into maps and drop other short tags,
/// so the template parses as plain YAML.
fn untag(text: &str) -> String {
    let get_att = Regex::new(r"!GetAtt\s+([A-Za-z0-9]+)\.[A-Za-z0-9.]+").expect("regex");
    let reference = Regex::new(r"!Ref\s+([A-Za-z0-9:]+)").expect("regex");
    let other = Regex::new(r"![A-Za-z]+\s").expect("regex");
    let text = get_att.replace_all(text, "{Ref: $1}");
    let text = reference.replace_all(&text, "{Ref: $1}");
    other.replace_all(&text, "").into_owned()
}

fn cfn_ref(value: &Value) -> Option<String> {
    if let Some(name) = value.get("Ref").and_then(Value::as_str) {
        return Some(name.to_owned());
    }
    match value.get("Fn::GetAtt") {
        Some(Value::Array(parts)) => parts.first()?.as_str().map(str::to_owned),
        Some(Value::String(text)) => text.split('.').next().map(str::to_owned),
        _ => None,
    }
}

fn cfn_refs(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(cfn_ref)
        .collect()
}

/// What the read-only AWS CLI calls returned for one account and region.
/// Each field is the JSON document of one call; a missing one is skipped.
#[derive(Debug, Default)]
pub struct AwsAccount {
    /// `ec2 describe-security-groups`.
    pub security_groups: Option<Value>,
    /// `ecs describe-services`, one document per batch.
    pub ecs_services: Vec<Value>,
    /// `lambda list-functions`.
    pub lambda_functions: Option<Value>,
    /// `rds describe-db-instances`.
    pub db_instances: Option<Value>,
    /// `rds describe-db-clusters`.
    pub db_clusters: Option<Value>,
    /// `elasticache describe-cache-clusters`.
    pub cache_clusters: Option<Value>,
    /// `xray get-service-graph`, the calls that really happened.
    pub service_graph: Option<Value>,
    /// ECS task descriptions from the current account.
    pub ecs_tasks: Vec<Value>,
    /// EC2 network interfaces and their VPCs.
    pub network_interfaces: Option<Value>,
    /// Default-format VPC log events from CloudWatch Logs.
    pub flow_events: Option<Value>,
}

/// Records from a live AWS account: the same security-group ties as from
/// Terraform, plus the calls X-Ray saw. `env` names the environment of the
/// account (`prod`); without it, an env word in the service name is used.
#[must_use]
pub fn account_records(account: &AwsAccount, env: Option<&str>) -> Vec<RawRecord> {
    let mut model = Model::default();
    let list = |value: Option<&Value>, key: &str| -> Vec<Value> {
        value
            .and_then(|value| value.get(key))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    for group in list(account.security_groups.as_ref(), "SecurityGroups") {
        let Some(id) = group.get("GroupId").and_then(Value::as_str) else {
            continue;
        };
        let tags: BTreeMap<String, String> = group
            .get("Tags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|tag| {
                Some((
                    tag.get("Key")?.as_str()?.to_owned(),
                    tag.get("Value")?.as_str()?.to_owned(),
                ))
            })
            .collect();
        let own = group
            .get("GroupName")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| id.to_owned());
        if own == "default" {
            continue;
        }
        let hint = tag_hint(&tags).unwrap_or_else(|| own.clone());
        model.groups.insert(id.to_owned(), hint);
        model.names.insert(id.to_owned(), own);
        model.defined.insert(id.to_owned());
        for (field, inbound) in [("IpPermissions", true), ("IpPermissionsEgress", false)] {
            for permission in group
                .get(field)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                for other in
                    super::cloud::strings(permission.get("UserIdGroupPairs"), Some("GroupId"))
                {
                    if inbound {
                        model.rules.insert((other, id.to_owned()));
                    } else {
                        model.rules.insert((id.to_owned(), other));
                    }
                }
            }
        }
    }
    for batch in &account.ecs_services {
        for service in list(Some(batch), "services") {
            let refs = super::cloud::strings(
                service.pointer("/networkConfiguration/awsvpcConfiguration/securityGroups"),
                None,
            );
            let name = service
                .get("serviceName")
                .and_then(Value::as_str)
                .map(str::to_owned);
            attach(&mut model, refs, name.map(Owner::Service));
        }
    }
    for function in list(account.lambda_functions.as_ref(), "Functions") {
        let refs = super::cloud::strings(function.pointer("/VpcConfig/SecurityGroupIds"), None);
        let name = function
            .get("FunctionName")
            .and_then(Value::as_str)
            .map(str::to_owned);
        attach(&mut model, refs, name.map(Owner::Service));
    }
    // A cluster member names its cluster, so instances of one Aurora cluster
    // become one database.
    for cluster in list(account.db_clusters.as_ref(), "DBClusters") {
        let refs =
            super::cloud::strings(cluster.get("VpcSecurityGroups"), Some("VpcSecurityGroupId"));
        let name = cluster
            .get("DBClusterIdentifier")
            .and_then(Value::as_str)
            .map(str::to_owned);
        attach(&mut model, refs, name.map(Owner::Database));
    }
    for instance in list(account.db_instances.as_ref(), "DBInstances") {
        let refs = super::cloud::strings(
            instance.get("VpcSecurityGroups"),
            Some("VpcSecurityGroupId"),
        );
        let name = instance
            .get("DBClusterIdentifier")
            .or(instance.get("DBInstanceIdentifier"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        attach(&mut model, refs, name.map(Owner::Database));
    }
    for cache in list(account.cache_clusters.as_ref(), "CacheClusters") {
        let refs = super::cloud::strings(cache.get("SecurityGroups"), Some("SecurityGroupId"));
        let name = cache
            .get("ReplicationGroupId")
            .or(cache.get("CacheClusterId"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        attach(&mut model, refs, name.map(Owner::Database));
    }
    let mut records = emit("aws", &model);
    if let Some(graph) = &account.service_graph {
        records.extend(xray_records(graph, env));
    }
    if account.flow_events.is_some() && account.network_interfaces.is_some() {
        // An empty successful scan must replace the previous flow file too.
        records.push(RawRecord {
            collector: "aws_flow_logs".into(),
            kind: "network_flow_checkpoint".into(),
            payload: serde_json::json!({}),
        });
        records.extend(super::flow_logs::aws_flows(account, env));
    }
    records
}

/// Calls in an X-Ray service graph. `client` nodes are callers outside
/// the estate and are skipped; AWS data stores become resource nodes.
fn xray_records(graph: &Value, env: Option<&str>) -> Vec<RawRecord> {
    use super::cloud::{Evidence, Seen, Target, count, observed};
    let services: Vec<&Value> = graph
        .get("Services")
        .and_then(Value::as_array)
        .map(|items| items.iter().collect())
        .unwrap_or_default();
    let by_ref: BTreeMap<i64, (&str, &str)> = services
        .iter()
        .filter_map(|service| {
            Some((
                service.get("ReferenceId")?.as_i64()?,
                (
                    service.get("Name")?.as_str()?,
                    service.get("Type").and_then(Value::as_str).unwrap_or(""),
                ),
            ))
        })
        .collect();
    let mut records = Vec::new();
    for service in &services {
        let (Some(source), kind) = (
            service.get("Name").and_then(Value::as_str),
            service.get("Type").and_then(Value::as_str).unwrap_or(""),
        ) else {
            continue;
        };
        if kind == "client"
            || xray_target(source, kind).is_none_or(|t| !matches!(t, Target::Service(_)))
        {
            continue;
        }
        for edge in service
            .get("Edges")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some((name, kind)) = edge
                .get("ReferenceId")
                .and_then(Value::as_i64)
                .and_then(|id| by_ref.get(&id))
            else {
                continue;
            };
            let Some(target) = xray_target(name, kind) else {
                continue;
            };
            let stats = edge.get("SummaryStatistics");
            let calls = count(stats.and_then(|s| s.get("TotalCount")));
            let errors = count(stats.and_then(|s| s.pointer("/ErrorStatistics/TotalCount")))
                + count(stats.and_then(|s| s.pointer("/FaultStatistics/TotalCount")));
            records.extend(observed(&Seen {
                source,
                target,
                calls,
                errors,
                env,
                window: "6h",
                evidence: Evidence::Xray,
            }));
        }
    }
    records
}

/// What an X-Ray node is. `None` for nodes that add nothing (S3, STS).
fn xray_target(name: &str, kind: &str) -> Option<super::cloud::Target> {
    use super::cloud::Target;
    let resource = |kind| {
        Some(Target::Resource(
            kind,
            name.split('.').next().unwrap_or(name).to_owned(),
        ))
    };
    match kind {
        "client" => None,
        k if k.starts_with("AWS::RDS")
            || k.starts_with("AWS::DynamoDB")
            || k.starts_with("AWS::ElastiCache")
            || k.starts_with("Database::") =>
        {
            resource("database")
        }
        "AWS::SQS::Queue" | "AWS::SQS" => resource("queue"),
        "AWS::SNS::Topic" | "AWS::SNS" => resource("topic"),
        "remote" if name.contains('.') => Some(Target::External(name.to_owned())),
        "remote" | "" => Some(Target::Service(name.to_owned())),
        k if k.starts_with("AWS::ECS")
            || k.starts_with("AWS::EC2")
            || k.starts_with("AWS::EKS")
            || k.starts_with("AWS::Lambda")
            || k.starts_with("AWS::ElasticBeanstalk")
            || k.starts_with("AWS::ApiGateway")
            || k.starts_with("AWS::AppRunner")
            || k.starts_with("AWS::StepFunctions") =>
        {
            Some(Target::Service(name.to_owned()))
        }
        _ => None,
    }
}

/// Files read by [`records_from_aws_dir`], named after the CLI call.
pub const AWS_FIXTURE_FILES: [&str; 10] = [
    "security-groups.json",
    "ecs-services.json",
    "lambda-functions.json",
    "db-instances.json",
    "db-clusters.json",
    "cache-clusters.json",
    "service-graph.json",
    "ecs-tasks.json",
    "network-interfaces.json",
    "flow-events.json",
];

/// Read saved AWS CLI output from a directory (for tests and dry runs).
///
/// # Errors
///
/// Returns an error when a file exists but is not JSON.
pub fn records_from_aws_dir(
    dir: &std::path::Path,
    env: Option<&str>,
) -> Result<Vec<RawRecord>, crate::error::Error> {
    use super::cloud::read_json;
    let account = AwsAccount {
        security_groups: read_json(dir, "security-groups.json")?,
        ecs_services: read_json(dir, "ecs-services.json")?.into_iter().collect(),
        lambda_functions: read_json(dir, "lambda-functions.json")?,
        db_instances: read_json(dir, "db-instances.json")?,
        db_clusters: read_json(dir, "db-clusters.json")?,
        cache_clusters: read_json(dir, "cache-clusters.json")?,
        service_graph: read_json(dir, "service-graph.json")?,
        ecs_tasks: read_json(dir, "ecs-tasks.json")?.into_iter().collect(),
        network_interfaces: read_json(dir, "network-interfaces.json")?,
        flow_events: read_json(dir, "flow-events.json")?,
    };
    Ok(account_records(&account, env))
}

/// How to reach the account. Profile and regions are names, not secrets.
#[derive(Debug, Clone, Default)]
pub struct AwsLive {
    /// `--profile` for the AWS CLI. `None` uses its default chain.
    pub profile: Option<String>,
    /// Regions to read. Empty uses the CLI's default region.
    pub regions: Vec<String>,
    /// Environment of this account.
    pub env: Option<String>,
    /// Also read the X-Ray service graph.
    pub xray: bool,
    /// Optional CloudWatch log group containing VPC flow logs.
    pub flow_log_group: Option<String>,
    /// Discover active VPC flow logs in CloudWatch when no group is specified.
    pub flow_logs: bool,
}

/// Read a live AWS account with the `aws` CLI. Only list and describe calls
/// are made. A call other than `describe-security-groups` that fails (no
/// permission, service not used) becomes a warning.
///
/// # Errors
///
/// Returns an error when the CLI is missing or security groups cannot be read.
pub fn records_from_aws(
    live: &AwsLive,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(Vec<RawRecord>, Vec<String>), crate::error::Error> {
    use super::cloud::{run_json, strings};
    let mut records = Vec::new();
    let mut warnings = Vec::new();
    let mut flow_complete = true;
    let mut no_flow_groups = Vec::new();
    let flow_enabled = live.flow_logs || live.flow_log_group.is_some();
    let regions: Vec<Option<&str>> = if live.regions.is_empty() {
        vec![None]
    } else {
        live.regions
            .iter()
            .map(|region| Some(region.as_str()))
            .collect()
    };
    for region in regions {
        let call = |parts: &[&str]| -> Result<Value, crate::error::Error> {
            let mut args: Vec<String> = parts.iter().map(|part| (*part).to_owned()).collect();
            args.extend(["--output".to_owned(), "json".to_owned()]);
            if let Some(profile) = &live.profile {
                args.extend(["--profile".to_owned(), profile.clone()]);
            }
            if let Some(region) = region {
                args.extend(["--region".to_owned(), region.to_owned()]);
            }
            run_json("aws", &args)
        };
        let mut soft = |parts: &[&str]| match call(parts) {
            Ok(value) => Some(value),
            Err(err) => {
                warnings.push(format!("aws {}: {err}", region.unwrap_or("default region")));
                None
            }
        };
        let mut account = AwsAccount {
            security_groups: Some(call(&["ec2", "describe-security-groups"])?),
            ..AwsAccount::default()
        };
        let clusters = soft(&["ecs", "list-clusters"]);
        if flow_enabled
            && clusters
                .as_ref()
                .and_then(|v| v.get("clusterArns"))
                .and_then(Value::as_array)
                .is_none()
        {
            flow_complete = false;
        }
        if let Some(clusters) = clusters {
            for cluster in strings(clusters.get("clusterArns"), None) {
                if flow_enabled {
                    let tasks = soft(&[
                        "ecs",
                        "list-tasks",
                        "--cluster",
                        &cluster,
                        "--desired-status",
                        "RUNNING",
                    ]);
                    if tasks
                        .as_ref()
                        .and_then(|v| v.get("taskArns"))
                        .and_then(Value::as_array)
                        .is_none()
                    {
                        flow_complete = false;
                    }
                    if let Some(tasks) = tasks {
                        let arns = strings(tasks.get("taskArns"), None);
                        for batch in arns.chunks(100) {
                            let mut parts = vec![
                                "ecs",
                                "describe-tasks",
                                "--cluster",
                                cluster.as_str(),
                                "--tasks",
                            ];
                            parts.extend(batch.iter().map(String::as_str));
                            let result = soft(&parts);
                            if result
                                .as_ref()
                                .and_then(|v| v.get("tasks"))
                                .and_then(Value::as_array)
                                .is_none()
                                || result
                                    .as_ref()
                                    .and_then(|v| v.get("failures"))
                                    .and_then(Value::as_array)
                                    .is_some_and(|v| !v.is_empty())
                            {
                                flow_complete = false;
                            }
                            account.ecs_tasks.extend(result);
                        }
                    }
                }
                let Some(services) = soft(&["ecs", "list-services", "--cluster", &cluster]) else {
                    continue;
                };
                let arns = strings(services.get("serviceArns"), None);
                for batch in arns.chunks(10) {
                    let mut parts = vec![
                        "ecs",
                        "describe-services",
                        "--cluster",
                        cluster.as_str(),
                        "--services",
                    ];
                    parts.extend(batch.iter().map(String::as_str));
                    account.ecs_services.extend(soft(&parts));
                }
            }
        }
        account.lambda_functions = soft(&["lambda", "list-functions"]);
        account.db_instances = soft(&["rds", "describe-db-instances"]);
        account.db_clusters = soft(&["rds", "describe-db-clusters"]);
        account.cache_clusters = soft(&["elasticache", "describe-cache-clusters"]);
        if live.xray {
            let end = now.timestamp().to_string();
            let start = (now - chrono::Duration::hours(6)).timestamp().to_string();
            account.service_graph = soft(&[
                "xray",
                "get-service-graph",
                "--start-time",
                &start,
                "--end-time",
                &end,
            ]);
        }
        if flow_enabled {
            let mut groups = std::collections::BTreeSet::new();
            if let Some(group) = &live.flow_log_group {
                groups.insert(group.clone());
            } else {
                let discovered = soft(&["ec2", "describe-flow-logs"]);
                if let Some(logs) = discovered
                    .as_ref()
                    .and_then(|v| v.get("FlowLogs"))
                    .and_then(Value::as_array)
                {
                    let default_format = "${version} ${account-id} ${interface-id} ${srcaddr} ${dstaddr} ${srcport} ${dstport} ${protocol} ${packets} ${bytes} ${start} ${end} ${action} ${log-status}";
                    let mut group_formats = std::collections::BTreeMap::<String, bool>::new();
                    for log in logs {
                        if log.get("FlowLogStatus").and_then(Value::as_str) != Some("ACTIVE")
                            || log.get("LogDestinationType").and_then(Value::as_str)
                                != Some("cloud-watch-logs")
                        {
                            continue;
                        }
                        if let Some(group) = log.get("LogGroupName").and_then(Value::as_str) {
                            let valid =
                                log.get("LogFormat")
                                    .and_then(Value::as_str)
                                    .is_some_and(|s| {
                                        s.split_whitespace().eq(default_format.split_whitespace())
                                    });
                            group_formats
                                .entry(group.to_owned())
                                .and_modify(|v| *v &= valid)
                                .or_insert(valid);
                        } else {
                            flow_complete = false;
                        }
                    }
                    for (group, valid) in group_formats {
                        if valid {
                            groups.insert(group);
                        } else {
                            flow_complete = false;
                        }
                    }
                } else {
                    flow_complete = false;
                }
            }
            if groups.is_empty() {
                no_flow_groups.push(region.unwrap_or("default region").to_owned());
            }
            account.network_interfaces = soft(&["ec2", "describe-network-interfaces"]);
            if account
                .network_interfaces
                .as_ref()
                .and_then(|v| v.get("NetworkInterfaces"))
                .and_then(Value::as_array)
                .is_none()
            {
                flow_complete = false;
            }
            let start = ((now - chrono::Duration::hours(6)).timestamp_millis()).to_string();
            let end = now.timestamp_millis().to_string();
            let mut events = Vec::new();
            for group in &groups {
                let result = soft(&[
                    "logs",
                    "filter-log-events",
                    "--log-group-name",
                    group,
                    "--start-time",
                    &start,
                    "--end-time",
                    &end,
                    "--max-items",
                    "10000",
                ]);
                match result {
                    Some(value)
                        if value.get("NextToken").is_none() && value.get("nextToken").is_none() =>
                    {
                        if let Some(items) = value.get("events").and_then(Value::as_array) {
                            events.extend(items.iter().cloned());
                        } else {
                            flow_complete = false;
                        }
                    }
                    _ => {
                        flow_complete = false;
                    }
                }
            }
            account.flow_events = Some(serde_json::json!({"events": events}));
        }
        records.extend(account_records(&account, live.env.as_deref()));
    }
    if flow_enabled && flow_complete {
        for region in no_flow_groups {
            warnings.push(format!("aws flow logs: no eligible active CloudWatch log groups in {region}; check VPC Flow Logs and log format"));
        }
    }
    if flow_enabled && !flow_complete {
        records.retain(|record| record.collector != "aws_flow_logs");
        warnings.push(
            "aws flow logs: an input was incomplete; any previous flow records are retained".into(),
        );
    }
    Ok((records, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(records: &[RawRecord]) -> Vec<String> {
        records
            .iter()
            .filter(|record| record.kind == "candidate_call")
            .map(|record| {
                format!(
                    "{} -> {} {}",
                    record.payload["source"].as_str().unwrap(),
                    record.payload["destination"].as_str().unwrap(),
                    record.payload["destination_kind"]
                        .as_str()
                        .unwrap_or("service")
                )
            })
            .collect()
    }

    #[test]
    fn terraform_rules_follow_attachments_and_names() {
        let text = r#"
resource "aws_security_group" "api" {
  name = "${var.env}-orders-api-sg"
  ingress {
    from_port       = 443
    security_groups = [aws_security_group.lb.id]
  }
  ingress {
    cidr_blocks = ["10.0.0.0/8"]
  }
}
resource "aws_security_group" "lb" {
  tags = {
    app = "edge"
  }
}
resource "aws_security_group" "db" {
  name = "orders-db-sg"
}
resource "aws_vpc_security_group_ingress_rule" "api_to_db" {
  security_group_id            = aws_security_group.db.id
  referenced_security_group_id = aws_security_group.api.id
}
resource "aws_db_instance" "orders" {
  identifier             = "orders-db"
  vpc_security_group_ids = [aws_security_group.db.id]
}
"#;
        let records = terraform_records("group/infra", text);
        assert_eq!(
            pairs(&records),
            [
                "orders-api-sg -> orders-db database",
                "edge -> orders-api-sg service"
            ]
        );
    }

    #[test]
    fn cloudformation_short_tags_are_read() {
        let text = "Resources:\n  ApiSg:\n    Type: AWS::EC2::SecurityGroup\n    Properties:\n      GroupDescription: !Sub '${Env} api'\n      SecurityGroupIngress:\n        - SourceSecurityGroupId: !GetAtt WorkerSg.GroupId\n          FromPort: 443\n  WorkerSg:\n    Type: AWS::EC2::SecurityGroup\n    Properties:\n      GroupName: worker-sg\n  Worker:\n    Type: AWS::ECS::Service\n    Properties:\n      ServiceName: report-worker\n      NetworkConfiguration:\n        AwsvpcConfiguration:\n          SecurityGroups:\n            - !Ref WorkerSg\n";
        let records = cloudformation_records("group/app", text);
        assert_eq!(pairs(&records), ["report-worker -> apisg service"]);
    }

    fn images(records: &[RawRecord]) -> Vec<String> {
        records
            .iter()
            .filter(|record| record.kind == "workload_image")
            .map(|record| {
                format!(
                    "{} runs {}",
                    record.payload["workload"].as_str().unwrap(),
                    record.payload["images"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|image| image.as_str().unwrap())
                        .collect::<Vec<_>>()
                        .join(",")
                )
            })
            .collect()
    }

    #[test]
    fn terraform_services_report_the_images_they_run() {
        let text = r#"
resource "aws_ecr_repository" "orders" {
  name = "team/orders-py"
}
resource "aws_ecs_task_definition" "api" {
  family = "api-task"
  container_definitions = jsonencode([
    { name = "agent", image = "public.ecr.aws/datadog/agent:7" },
    { name = "app", image = "${aws_ecr_repository.orders.repository_url}:${var.tag}" }
  ])
}
resource "aws_ecs_service" "api" {
  name            = "legacy-api"
  task_definition = aws_ecs_task_definition.api.arn
}
resource "aws_lambda_function" "hook" {
  function_name = "hook-fn"
  image_uri     = "123.dkr.ecr.eu-west-1.amazonaws.com/notifications:1.2"
}
resource "aws_ecs_service" "dynamic" {
  name            = "dyn"
  task_definition = aws_ecs_task_definition.api.arn
}
"#;
        let records = terraform_records("group/infra", text);
        assert_eq!(
            images(&records),
            [
                "dyn runs agent,orders-py",
                "hook-fn runs notifications",
                "legacy-api runs agent,orders-py"
            ]
        );
    }

    #[test]
    fn cloudformation_services_report_the_images_they_run() {
        let text = "Resources:\n  Task:\n    Type: AWS::ECS::TaskDefinition\n    Properties:\n      ContainerDefinitions:\n        - Image: !Sub '${AWS::AccountId}.dkr.ecr.${AWS::Region}.amazonaws.com/orders-py:latest'\n        - Image: ${Registry}/${Name}:1\n  Svc:\n    Type: AWS::ECS::Service\n    Properties:\n      ServiceName: legacy-api\n      TaskDefinition: !Ref Task\n";
        let records = cloudformation_records("group/app", text);
        assert_eq!(images(&records), ["legacy-api runs orders-py"]);
    }

    fn messages(records: &[RawRecord]) -> Vec<String> {
        let mut found: Vec<String> = records
            .iter()
            .filter_map(|record| match record.kind.as_str() {
                "messaging" => Some(format!(
                    "{} {} {} {}",
                    record.payload["source"].as_str()?,
                    record.payload["direction"].as_str()?,
                    record.payload["kind"].as_str()?,
                    record.payload["name"].as_str()?
                )),
                "resource_link" => Some(format!(
                    "{} {} receives {} {}",
                    record.payload["from_kind"].as_str()?,
                    record.payload["from"].as_str()?,
                    record.payload["to_kind"].as_str()?,
                    record.payload["to"].as_str()?
                )),
                _ => None,
            })
            .collect();
        found.sort();
        found
    }

    #[test]
    fn terraform_queues_and_topics_connect_functions() {
        let text = r#"
resource "aws_sns_topic" "events" {
  name = "order-events"
}
resource "aws_sqs_queue" "billing" {
  name = "billing-jobs.fifo"
}
resource "aws_sns_topic_subscription" "to_billing" {
  topic_arn = aws_sns_topic.events.arn
  protocol  = "sqs"
  endpoint  = aws_sqs_queue.billing.arn
}
resource "aws_lambda_function" "billing" {
  function_name = "billing-fn"
}
resource "aws_lambda_event_source_mapping" "billing" {
  event_source_arn = aws_sqs_queue.billing.arn
  function_name    = aws_lambda_function.billing.arn
}
resource "aws_lambda_function" "checkout" {
  function_name = "checkout-fn"
  environment {
    variables = { TOPIC = aws_sns_topic.events.arn, QUEUE = aws_sqs_queue.billing.url }
  }
}
"#;
        let records = terraform_records("group/infra", text);
        assert_eq!(
            messages(&records),
            [
                "billing-fn consume queue billing-jobs",
                "checkout-fn publish queue billing-jobs",
                "checkout-fn publish topic order-events",
                "queue billing-jobs receives topic order-events",
            ]
        );
    }

    #[test]
    fn cloudformation_queues_and_topics_connect_functions() {
        let text = "Resources:\n  Events:\n    Type: AWS::SNS::Topic\n    Properties:\n      TopicName: order-events\n  Jobs:\n    Type: AWS::SQS::Queue\n    Properties:\n      QueueName: billing-jobs\n  Sub:\n    Type: AWS::SNS::Subscription\n    Properties:\n      TopicArn: !Ref Events\n      Protocol: sqs\n      Endpoint: !GetAtt Jobs.Arn\n  Worker:\n    Type: AWS::Lambda::Function\n    Properties:\n      FunctionName: billing-fn\n  Map:\n    Type: AWS::Lambda::EventSourceMapping\n    Properties:\n      EventSourceArn: !GetAtt Jobs.Arn\n      FunctionName: !Ref Worker\n";
        let records = cloudformation_records("group/app", text);
        assert_eq!(
            messages(&records),
            [
                "billing-fn consume queue billing-jobs",
                "queue billing-jobs receives topic order-events",
            ]
        );
    }

    #[test]
    fn terraform_environment_names_the_databases_a_service_calls() {
        let text = r#"
resource "aws_db_instance" "orders" {
  identifier = "orders-db"
}
resource "aws_elasticache_cluster" "cache" {
  cluster_id = "orders-cache"
}
resource "aws_ecs_task_definition" "api" {
  container_definitions = jsonencode([{
    image = "orders-api:1"
    environment = [
      { name = "DATABASE_HOST", value = aws_db_instance.orders.address },
      { name = "CACHE", value = "orders-cache.abc123.0001.euw1.cache.amazonaws.com" }
    ]
  }])
}
resource "aws_ecs_service" "api" {
  name            = "orders-api"
  task_definition = aws_ecs_task_definition.api.arn
}
resource "aws_lambda_function" "report" {
  function_name = "report-fn"
  environment {
    variables = { DB = "legacy-db.cabc.eu-west-1.rds.amazonaws.com" }
  }
}
"#;
        let records = terraform_records("group/infra", text);
        assert_eq!(
            pairs(&records),
            [
                "orders-api -> orders-cache database",
                "orders-api -> orders-db database",
                "report-fn -> legacy-db database"
            ]
        );
    }

    #[test]
    fn cloudformation_environment_names_the_databases_a_service_calls() {
        let text = "Resources:\n  Db:\n    Type: AWS::RDS::DBInstance\n    Properties:\n      DBInstanceIdentifier: orders-db\n  Task:\n    Type: AWS::ECS::TaskDefinition\n    Properties:\n      ContainerDefinitions:\n        - Image: orders-api\n          Environment:\n            - Name: DB\n              Value: !GetAtt Db.Endpoint.Address\n  Svc:\n    Type: AWS::ECS::Service\n    Properties:\n      ServiceName: orders-api\n      TaskDefinition: !Ref Task\n";
        let records = cloudformation_records("group/app", text);
        assert_eq!(pairs(&records), ["orders-api -> orders-db database"]);
    }

    #[test]
    fn local_modules_are_expanded_once_per_call() {
        let module = r#"
variable "name" {}
variable "db" {
  default = "shared-db"
}
resource "aws_security_group" "svc" {
  name = "${var.name}-sg"
}
resource "aws_security_group" "database" {
  name = "${var.db}-sg"
  ingress {
    security_groups = [aws_security_group.svc.id]
  }
}
resource "aws_db_instance" "main" {
  identifier             = var.db
  vpc_security_group_ids = [aws_security_group.database.id]
}
resource "aws_ecs_service" "svc" {
  name = var.name
  network_configuration {
    security_groups = [aws_security_group.svc.id]
  }
}
"#;
        let root = r#"
module "orders" {
  source = "./modules/service"
  name   = "orders-api"
  db     = "orders-db"
}
module "billing" {
  source = "../infra/modules/service"
  name   = "billing-api"
}
module "remote" {
  source = "terraform-aws-modules/vpc/aws"
  name   = "ignored"
}
"#;
        let files = vec![
            ("infra/main.tf".to_owned(), root.replace("../infra/", "./")),
            (
                "infra/modules/service/main.tf".to_owned(),
                module.to_owned(),
            ),
        ];
        let records = terraform_records_from_files("group/infra", &files);
        assert_eq!(
            pairs(&records),
            [
                "billing-api -> shared-db database",
                "orders-api -> orders-db database"
            ]
        );
    }

    #[test]
    fn modules_called_but_missing_leave_the_rest_alone() {
        let files = vec![(
            "main.tf".to_owned(),
            "module \"x\" {\n  source = \"./gone\"\n}\nresource \"aws_db_instance\" \"d\" {\n  identifier = \"d-db\"\n}\n".to_owned(),
        )];
        assert!(terraform_records_from_files("g/p", &files).is_empty());
    }

    #[test]
    fn plain_string_locals_name_services_and_modules() {
        let root = r#"
locals {
  app  = "orders-api"
  full = "${local.app}-svc"
  dyn  = "ex-${basename(path.cwd)}"
}
module "svc" {
  source = "./modules/service"
  name   = local.full
}
module "other" {
  source = "./modules/service"
  name   = local.dyn
}
"#;
        let module = r#"
resource "aws_db_instance" "db" {
  identifier = "${var.name}-db"
}
resource "aws_ecs_service" "svc" {
  name = var.name
}
resource "aws_security_group" "sg" {
  name = "${var.name}-sg"
}
resource "aws_vpc_security_group_ingress_rule" "r" {
  security_group_id            = aws_security_group.sg.id
  referenced_security_group_id = aws_security_group.sg.id
}
"#;
        let files = vec![
            ("main.tf".to_owned(), root.to_owned()),
            ("modules/service/main.tf".to_owned(), module.to_owned()),
        ];
        let names: Vec<String> = terraform_records_from_files("g/infra", &files)
            .iter()
            .filter(|record| record.kind == "resource")
            .map(|record| record.payload["name"].as_str().unwrap().to_owned())
            .collect();
        assert!(names.contains(&"orders-api-svc-sg".to_owned()), "{names:?}");
    }
}
