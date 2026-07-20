using Oe.Domain.Entities;

namespace Oe.Application.Orders;

public sealed record SubmitOrderCommand(Guid OrderId);

public interface ISubmitOrderHandler
{
    Task<Order> HandleAsync(SubmitOrderCommand command, CancellationToken ct = default);
}
