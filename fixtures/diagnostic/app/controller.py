"""HTTP-facing payment controller."""

from app.service import PaymentService


class PaymentController:
    """Controller for payment requests."""

    def __init__(self, service: PaymentService):
        self.service = service

    def create_payment(self, payment_id: str) -> str:
        return self.service.charge(payment_id)
