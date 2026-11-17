import boto3
import requests

sns = boto3.client("sns")


def order_placed(order):
    # Tell everyone who cares, then ask billing to charge.
    sns.publish(TopicArn="arn:aws:sns:eu-west-1:123456789012:order-events", Message=order)
    requests.post("http://billing-ex:4000/charge", json=order)
