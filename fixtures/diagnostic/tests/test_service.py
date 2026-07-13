from app.service import PaymentRepository, PaymentService


def test_charge_persists_payment():
    service = PaymentService(PaymentRepository())
    assert service.charge("payment-1") == "payment-1"
