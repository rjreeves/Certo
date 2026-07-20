using Oe.Domain.Entities;

namespace Oe.Application.Orders;

public sealed record AddOrderLineCommand(
    Guid    OrderId,
    Guid    ProductId,
    int     Quantity,
    decimal DiscountPercent = 0);

public sealed record ApplyDiscountCommand(
    Guid    OrderLineId,
    decimal DiscountPercent);

public interface IOrderLineHandler
{
    Task<OrderLine> AddLineAsync(AddOrderLineCommand command, CancellationToken ct = default);
    Task<OrderLine> ApplyDiscountAsync(ApplyDiscountCommand command, CancellationToken ct = default);
}
