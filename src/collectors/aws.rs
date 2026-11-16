//! AWS security groups from Terraform and CloudFormation.
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
}

/// Records from all Terraform files of one project, read together so that
/// references across files resolve.
#[must_use]
pub fn terraform_records(project_path: &str, text: &str) -> Vec<RawRecord> {
    if !text.contains("aws_security_group") {
        return Vec::new();
    }
    let mut model = Model::default();
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
            ("resource", "aws_ecs_service") => {
                let name = tf_string(&top, "name");
                let refs: Vec<String> = nested(body, "network_configuration")
                    .iter()
                    .flat_map(|block| sg_refs(&tf_value(block, "security_groups")))
                    .collect();
                attach(&mut model, refs, name.map(Owner::Service));
            }
            ("resource", "aws_lambda_function") => {
                let name = tf_string(&top, "function_name");
                let refs: Vec<String> = nested(body, "vpc_config")
                    .iter()
                    .flat_map(|block| sg_refs(&tf_value(block, "security_group_ids")))
                    .collect();
                attach(&mut model, refs, name.map(Owner::Service));
            }
            ("resource", "aws_db_instance" | "aws_rds_cluster") => {
                let name =
                    tf_string(&top, "identifier").or_else(|| tf_string(&top, "cluster_identifier"));
                let refs = sg_refs(&tf_value(&top, "vpc_security_group_ids"));
                attach(&mut model, refs, name.map(Owner::Database));
            }
            ("resource", "aws_elasticache_cluster" | "aws_elasticache_replication_group") => {
                let name = tf_string(&top, "cluster_id")
                    .or_else(|| tf_string(&top, "replication_group_id"));
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
            "AWS::ECS::Service" => {
                let refs = cfn_refs(
                    field("NetworkConfiguration")
                        .get("AwsvpcConfiguration")
                        .and_then(|config| config.get("SecurityGroups")),
                );
                let name = field("ServiceName").as_str().unwrap_or(id).to_owned();
                attach(&mut model, refs, Some(Owner::Service(name)));
            }
            "AWS::Lambda::Function" => {
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
                attach(&mut model, refs, Some(Owner::Database(name)));
            }
            _ => {}
        }
    }
    emit(project_path, &model)
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
    records
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

/// The body with every nested `{...}` removed.
fn top_level(body: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for c in body.chars() {
        match c {
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
}
