using Oe.Application.Orders;
using Oe.Domain.Entities;

namespace Oe.Api.Endpoints;

public static class OrderEndpoints
{
    public static IEndpointRouteBuilder MapOrderEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/orders").RequireAuthorization();

        group.MapPost("{id:guid}/submit", SubmitOrder)
             .WithName("SubmitOrder")
             .Produces<Order>()
             .ProducesValidationProblem(422);

        group.MapPost("{id:guid}/lines", AddLine)
             .WithName("AddOrderLine")
             .Produces<OrderLine>(201);

        group.MapPatch("{orderId:guid}/lines/{lineId:guid}/discount", ApplyDiscount)
             .WithName("ApplyDiscount")
             .Produces<OrderLine>()
             .ProducesValidationProblem(422);

        return app;
    }

    private static async Task<IResult> SubmitOrder(
        Guid id,
        ISubmitOrderHandler handler,
        CancellationToken ct)
    {
        var order = await handler.HandleAsync(new SubmitOrderCommand(id), ct);
        return Results.Ok(order);
    }

    private static async Task<IResult> AddLine(
        Guid id,
        AddOrderLineRequest req,
        IOrderLineHandler handler,
        CancellationToken ct)
    {
        var line = await handler.AddLineAsync(
            new AddOrderLineCommand(id, req.ProductId, req.Quantity, req.DiscountPercent), ct);
        return Results.Created($"/orders/{id}/lines/{line.Id}", line);
    }

    private static async Task<IResult> ApplyDiscount(
        Guid orderId,
        Guid lineId,
        ApplyDiscountRequest req,
        IOrderLineHandler handler,
        CancellationToken ct)
    {
        var line = await handler.ApplyDiscountAsync(
            new ApplyDiscountCommand(lineId, req.DiscountPercent), ct);
        return Results.Ok(line);
    }

    private sealed record AddOrderLineRequest(Guid ProductId, int Quantity, decimal DiscountPercent = 0);
    private sealed record ApplyDiscountRequest(decimal DiscountPercent);
}
