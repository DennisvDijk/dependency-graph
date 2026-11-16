# Security groups for the shop on AWS. depgraph reads which group may reach
# which, and ties each group to its service by attachment, tag or name.

resource "aws_security_group" "orders" {
  name = "orders-py-prod-sg"

  ingress {
    from_port   = 443
    to_port     = 443
    protocol    = "tcp"
    cidr_blocks = ["10.0.0.0/8"]
  }
}

resource "aws_security_group" "edge" {
  name = "edge-public-sg"
  tags = {
    app = "edge-proxy-rs"
  }
}

resource "aws_vpc_security_group_ingress_rule" "edge_to_orders" {
  security_group_id            = aws_security_group.orders.id
  referenced_security_group_id = aws_security_group.edge.id
  from_port                    = 8080
  to_port                      = 8080
  ip_protocol                  = "tcp"
}

# The name says nothing; the ECS service it is attached to does.
resource "aws_security_group" "billing" {
  name = "payments-sg"
}

resource "aws_ecs_service" "billing" {
  name = "billing-ex"
  network_configuration {
    subnets         = var.private_subnets
    security_groups = [aws_security_group.billing.id]
  }
}

resource "aws_security_group" "keycloak" {
  name = "prod-keycloak"
}

resource "aws_security_group_rule" "billing_to_keycloak" {
  type                     = "ingress"
  security_group_id        = aws_security_group.keycloak.id
  source_security_group_id = aws_security_group.billing.id
  from_port                = 8443
  to_port                  = 8443
  protocol                 = "tcp"
}
