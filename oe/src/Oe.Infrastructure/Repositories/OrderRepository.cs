using Dapper;
using Npgsql;
using Oe.Application.Orders;
using Oe.Application.Shared;
using Oe.Domain.Entities;
using Oe.Domain.Enums;
using Oe.Domain.Exceptions;
using Oe.Infrastructure.Database;
using Oe.Infrastructure.Validation;

namespace Oe.Infrastructure.Repositories;

public sealed class OrderRepository(DbConnectionFactory db, ICurrentUser currentUser)
    : ISubmitOrderHandler, IOrderLineHandler
{
    // ------------------------------------------------------------------ //
    // Submit
    // ------------------------------------------------------------------ //

    public async Task<Order> HandleAsync(SubmitOrderCommand command, CancellationToken ct = default)
    {
        await using var conn = db.Create();
        await conn.OpenAsync(ct);
        await using var tx = await conn.BeginTransactionAsync(ct);

        await RuleContext.SetAsync(conn, tx, currentUser.Id, currentUser.Role, ct);

        // Snapshot available_credit from the customer onto the order record
        // so the validator JSONB sees it.
        await conn.ExecuteAsync(
            """
            UPDATE orders SET
                available_credit = (
                    SELECT credit_limit - outstanding_balance
                    FROM customers WHERE id = orders.customer_id
                )
            WHERE id = @id
            """,
            new { id = command.OrderId }, tx);

        var record = await conn.QuerySingleOrDefaultAsync<Order>(
            "SELECT * FROM orders WHERE id = @id",
            new { id = command.OrderId }, tx)
            ?? throw new KeyNotFoundException($"Order {command.OrderId} not found.");

        if (record.Status != OrderStatus.Draft)
            throw RuleViolationException.Single("ORDER_NOT_DRAFT", "Only draft orders can be submitted.");

        var validator = new PostgresValidator(conn, tx);
        await validator.ValidateAsync("preflight_orders_submit", record, ct);

        var updated = await conn.QuerySingleAsync<Order>(
            """
            UPDATE orders SET status = 'SUBMITTED', updated_at = NOW()
            WHERE id = @id
            RETURNING *
            """,
            new { id = command.OrderId }, tx);

        await tx.CommitAsync(ct);
        return updated;
    }

    // ------------------------------------------------------------------ //
    // Add line
    // ------------------------------------------------------------------ //

    public async Task<OrderLine> AddLineAsync(AddOrderLineCommand command, CancellationToken ct = default)
    {
        await using var conn = db.Create();
        await conn.OpenAsync(ct);
        await using var tx = await conn.BeginTransactionAsync(ct);

        await RuleContext.SetAsync(conn, tx, currentUser.Id, currentUser.Role, ct);

        var product = await conn.QuerySingleOrDefaultAsync<dynamic>(
            "SELECT unit_price, is_digital FROM products WHERE id = @id",
            new { id = command.ProductId }, tx)
            ?? throw new KeyNotFoundException($"Product {command.ProductId} not found.");

        decimal unitPrice      = product.unit_price;
        decimal subtotal       = unitPrice * command.Quantity;
        decimal discountAmount = Math.Round(subtotal * command.DiscountPercent / 100, 4);
        decimal total          = subtotal - discountAmount;

        var line = await conn.QuerySingleAsync<OrderLine>(
            """
            INSERT INTO order_lines
                (order_id, product_id, unit_price, quantity,
                 discount_percent, discount_amount, subtotal, total, is_digital)
            VALUES
                (@orderId, @productId, @unitPrice, @quantity,
                 @discountPercent, @discountAmount, @subtotal, @total, @isDigital)
            RETURNING *
            """,
            new
            {
                orderId         = command.OrderId,
                productId       = command.ProductId,
                unitPrice,
                quantity        = command.Quantity,
                discountPercent = command.DiscountPercent,
                discountAmount,
                subtotal,
                total,
                isDigital       = (bool)product.is_digital,
            }, tx);

        await tx.CommitAsync(ct);
        return line;
    }

    // ------------------------------------------------------------------ //
    // Apply discount
    // ------------------------------------------------------------------ //

    public async Task<OrderLine> ApplyDiscountAsync(ApplyDiscountCommand command, CancellationToken ct = default)
    {
        await using var conn = db.Create();
        await conn.OpenAsync(ct);
        await using var tx = await conn.BeginTransactionAsync(ct);

        await RuleContext.SetAsync(conn, tx, currentUser.Id, currentUser.Role, ct);

        var line = await conn.QuerySingleOrDefaultAsync<OrderLine>(
            "SELECT * FROM order_lines WHERE id = @id",
            new { id = command.OrderLineId }, tx)
            ?? throw new KeyNotFoundException($"Order line {command.OrderLineId} not found.");

        decimal discountAmount = Math.Round(line.UnitPrice * command.DiscountPercent / 100, 4);

        // Build the candidate record the validator will inspect.
        var candidate = new
        {
            line.Id,
            line.OrderId,
            line.ProductId,
            line.UnitPrice,
            line.Quantity,
            DiscountPercent = command.DiscountPercent,
            DiscountAmount  = discountAmount,
            line.Subtotal,
            Total           = line.Subtotal - discountAmount,
            line.IsDigital,
        };

        var validator = new PostgresValidator(conn, tx);
        await validator.ValidateAsync("preflight_order_lines_apply_discount", candidate, ct);

        var updated = await conn.QuerySingleAsync<OrderLine>(
            """
            UPDATE order_lines SET
                discount_percent = @discountPercent,
                discount_amount  = @discountAmount,
                total            = subtotal - @discountAmount,
                updated_at       = NOW()
            WHERE id = @id
            RETURNING *
            """,
            new
            {
                id              = command.OrderLineId,
                discountPercent = command.DiscountPercent,
                discountAmount,
            }, tx);

        await tx.CommitAsync(ct);
        return updated;
    }
}
