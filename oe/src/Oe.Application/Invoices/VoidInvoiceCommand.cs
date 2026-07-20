using Oe.Domain.Entities;

namespace Oe.Application.Invoices;

public sealed record VoidInvoiceCommand(Guid InvoiceId);

public interface IVoidInvoiceHandler
{
    Task<Invoice> HandleAsync(VoidInvoiceCommand command, CancellationToken ct = default);
}
