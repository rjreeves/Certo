using Dapper;
using Oe.Application.Invoices;
using Oe.Application.Shared;
using Oe.Domain.Entities;
using Oe.Domain.Enums;
using Oe.Infrastructure.Database;
using Oe.Infrastructure.Validation;

namespace Oe.Infrastructure.Repositories;

public sealed class InvoiceRepository(DbConnectionFactory db, ICurrentUser currentUser)
    : IVoidInvoiceHandler
{
    public async Task<Invoice> HandleAsync(VoidInvoiceCommand command, CancellationToken ct = default)
    {
        await using var conn = db.Create();
        await conn.OpenAsync(ct);
        await using var tx = await conn.BeginTransactionAsync(ct);

        await RuleContext.SetAsync(conn, tx, currentUser.Id, currentUser.Role, ct);

        var invoice = await conn.QuerySingleOrDefaultAsync<Invoice>(
            "SELECT * FROM invoices WHERE id = @id",
            new { id = command.InvoiceId }, tx)
            ?? throw new KeyNotFoundException($"Invoice {command.InvoiceId} not found.");

        var validator = new PostgresValidator(conn, tx);
        await validator.ValidateAsync("preflight_invoices_void", invoice, ct);

        var updated = await conn.QuerySingleAsync<Invoice>(
            """
            UPDATE invoices SET
                status     = 'VOIDED',
                voided_at  = NOW(),
                voided_by  = @voidedBy,
                updated_at = NOW()
            WHERE id = @id
            RETURNING *
            """,
            new { id = command.InvoiceId, voidedBy = currentUser.Id }, tx);

        await tx.CommitAsync(ct);
        return updated;
    }
}
