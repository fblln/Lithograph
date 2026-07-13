"""Payment service and persistence boundary."""


class PaymentRepository:
    """Persist completed charges."""

    def save(self, payment_id: str) -> str:
        return payment_id


class PaymentService:
    """Coordinate payment operations."""

    def __init__(self, repository: PaymentRepository):
        self.repository = repository

    def charge(self, payment_id: str) -> str:
        return self.repository.save(payment_id)
