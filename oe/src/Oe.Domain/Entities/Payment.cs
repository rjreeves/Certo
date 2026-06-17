using Oe.Domain.Enums;

namespace Oe.Domain.Entities;

public sealed class Payment
{
    public Guid           Id                { get; init; }
    public Guid           InvoiceId         { get; init; }
    public Guid           CustomerId        { get; init; }
    public PaymentStatus  Status            { get; init; }
    public decimal        Amount            { get; init; }
    public string         Currency          { get; init; } = "GBP";
    public decimal        AmountOutstanding { get; init; }
    public string?        Reference         { get; init; }
    public string?        Notes             { get; init; }
    public DateTimeOffset CreatedAt         { get; init; }
    public DateTimeOffset UpdatedAt         { get; init; }
}
