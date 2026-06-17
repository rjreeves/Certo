using Dapper;
using Oe.Application.Payments;
using Oe.Application.Shared;
using Oe.Domain.Entities;
using Oe.Infrastructure.Database;
using Oe.Infrastructure.Validation;

namespace Oe.Infrastructure.Repositories;

public sealed class PaymentRepository(DbConnectionFactory db, ICurrentUser currentUser)
    : ICreatePaymentHandler
{
    public async Task<Payment> HandleAsync(CreatePaymentCommand command, CancellationToken ct = default)
    {
        await using var conn = db.Create();
        await conn.OpenAsync(ct);
        await using var tx = await conn.BeginTransactionAsync(ct);

        await RuleContext.SetAsync(conn, tx, currentUser.Id, currentUser.Role, ct);

        var invoice = await conn.QuerySingleOrDefaultAsync<Invoice>(
            "SELECT * FROM invoices WHERE id = @id",
            new { id = command.InvoiceId }, tx)
            ?? throw new KeyNotFoundException($"Invoice {command.InvoiceId} not found.");

        // Build the candidate record: mix of command fields and invoice snapshot.
        var candidate = new
        {
            invoice.Id,
            invoice.OrderId,
            invoice.CustomerId,
            invoice.Status,
            Amount             = command.Amount,
            Currency           = command.Currency,
            AmountOutstanding  = invoice.AmountOutstanding,
        };

        var validator = new PostgresValidator(conn, tx);
        await validator.ValidateAsync("preflight_payments_create", candidate, ct);

        var payment = await conn.QuerySingleAsync<Payment>(
            """
            INSERT INTO payments
                (invoice_id, customer_id, amount, currency, amount_outstanding, reference)
            VALUES
                (@invoiceId, @customerId, @amount, @currency, @amountOutstanding, @reference)
            RETURNING *
            """,
            new
            {
                invoiceId         = command.InvoiceId,
                customerId        = invoice.CustomerId,
                amount            = command.Amount,
                currency          = command.Currency,
                amountOutstanding = invoice.AmountOutstanding,
                reference         = command.Reference,
            }, tx);

        await tx.CommitAsync(ct);
        return payment;
    }
}
