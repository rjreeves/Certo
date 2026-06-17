using Oe.Domain.Entities;

namespace Oe.Application.Payments;

public sealed record CreatePaymentCommand(
    Guid    InvoiceId,
    decimal Amount,
    string  Currency,
    string? Reference = null);

public interface ICreatePaymentHandler
{
    Task<Payment> HandleAsync(CreatePaymentCommand command, CancellationToken ct = default);
}
