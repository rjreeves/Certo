using Oe.Domain.Enums;

namespace Oe.Domain.Entities;

public sealed class Invoice
{
    public Guid           Id                { get; init; }
    public Guid           OrderId           { get; init; }
    public Guid           CustomerId        { get; init; }
    public InvoiceStatus  Status            { get; init; }
    public decimal        Amount            { get; init; }
    public decimal        AmountPaid        { get; init; }
    public decimal        AmountOutstanding { get; init; }
    public string         Currency          { get; init; } = "GBP";
    public DateTimeOffset? IssuedAt         { get; init; }
    public DateTimeOffset? DueAt            { get; init; }
    public DateTimeOffset? VoidedAt         { get; init; }
    public Guid?          VoidedBy          { get; init; }
    public DateTimeOffset CreatedAt         { get; init; }
    public DateTimeOffset UpdatedAt         { get; init; }
}
