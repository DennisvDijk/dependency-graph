resource "aws_sns_topic" "order_events" {
  name = "order-events"
}

resource "aws_sqs_queue" "notify" {
  name = "notify-jobs"
}

resource "aws_sns_topic_subscription" "notify" {
  topic_arn = aws_sns_topic.order_events.arn
  protocol  = "sqs"
  endpoint  = aws_sqs_queue.notify.arn
}

resource "aws_lambda_function" "notifications" {
  function_name = "notifications"
}

resource "aws_lambda_event_source_mapping" "notify" {
  event_source_arn = aws_sqs_queue.notify.arn
  function_name    = aws_lambda_function.notifications.arn
}
