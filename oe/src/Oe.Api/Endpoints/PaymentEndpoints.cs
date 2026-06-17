using Oe.Application.Payments;
using Oe.Domain.Entities;

namespace Oe.Api.Endpoints;

public static class PaymentEndpoints
{
    public static IEndpointRouteBuilder MapPaymentEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/payments").RequireAuthorization();

        group.MapPost("", CreatePayment)
             .WithName("CreatePayment")
             .Produces<Payment>(201)
             .ProducesValidationProblem(422);

        return app;
    }

    private static async Task<IResult> CreatePayment(
        CreatePaymentRequest req,
        ICreatePaymentHandler handler,
        CancellationToken ct)
    {
        var payment = await handler.HandleAsync(
            new CreatePaymentCommand(req.InvoiceId, req.Amount, req.Currency, req.Reference), ct);
        return Results.Created($"/payments/{payment.Id}", payment);
    }

    private sealed record CreatePaymentRequest(
        Guid    InvoiceId,
        decimal Amount,
        string  Currency,
        string? Reference = null);
}
