resource "aws_security_group" "orders_db" {
  name = "orders-db-sg"

  ingress {
    from_port       = 5432
    to_port         = 5432
    protocol        = "tcp"
    security_groups = [aws_security_group.orders.id]
  }
}

resource "aws_db_instance" "orders" {
  identifier             = "orders-db"
  engine                 = "postgres"
  vpc_security_group_ids = [aws_security_group.orders_db.id]
}

# No service called legacy-batch exists: reported in unresolved.csv, not drawn.
resource "aws_security_group" "legacy" {
  name = "legacy-batch-sg"

  egress {
    from_port       = 443
    to_port         = 443
    protocol        = "tcp"
    security_groups = [aws_security_group.orders.id]
  }
}
