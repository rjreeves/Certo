using Oe.Application.Invoices;
using Oe.Domain.Entities;

namespace Oe.Api.Endpoints;

public static class InvoiceEndpoints
{
    public static IEndpointRouteBuilder MapInvoiceEndpoints(this IEndpointRouteBuilder app)
    {
        var group = app.MapGroup("/invoices").RequireAuthorization();

        group.MapPost("{id:guid}/void", VoidInvoice)
             .WithName("VoidInvoice")
             .Produces<Invoice>()
             .ProducesValidationProblem(422);

        return app;
    }

    private static async Task<IResult> VoidInvoice(
        Guid id,
        IVoidInvoiceHandler handler,
        CancellationToken ct)
    {
        var invoice = await handler.HandleAsync(new VoidInvoiceCommand(id), ct);
        return Results.Ok(invoice);
    }
}
