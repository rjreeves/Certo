using Oe.Domain.Enums;

namespace Oe.Domain.Entities;

public sealed class Order
{
    public Guid         Id                  { get; init; }
    public Guid         CustomerId          { get; init; }
    public OrderStatus  Status              { get; init; }
    public Guid?        ShippingAddressId   { get; init; }
    public Guid?        BillingAddressId    { get; init; }
    public int          LineCount           { get; init; }
    public bool         LinesAllDigital     { get; init; }
    public decimal      Subtotal            { get; init; }
    public decimal      DiscountTotal       { get; init; }
    public decimal      Total               { get; init; }
    public decimal?     AvailableCredit     { get; init; }
    public string?      Notes               { get; init; }
    public DateTimeOffset CreatedAt         { get; init; }
    public DateTimeOffset UpdatedAt         { get; init; }
}

public sealed class OrderLine
{
    public Guid     Id              { get; init; }
    public Guid     OrderId         { get; init; }
    public Guid     ProductId       { get; init; }
    public decimal  UnitPrice       { get; init; }
    public int      Quantity        { get; init; }
    public decimal  DiscountPercent { get; init; }
    public decimal  DiscountAmount  { get; init; }
    public decimal  Subtotal        { get; init; }
    public decimal  Total           { get; init; }
    public bool     IsDigital       { get; init; }
    public DateTimeOffset CreatedAt { get; init; }
    public DateTimeOffset UpdatedAt { get; init; }
}
