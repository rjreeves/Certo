namespace RuleGenerator.Validators;

/// <summary>Context objects carrying entity data into validators.
/// Generated — do not edit.</summary>

public sealed class UserContext
{
    public string Email { get; init; }
    public string FirstName { get; init; }
    public string LastName { get; init; }
    public UserRole Role { get; init; }
    public bool Active { get; init; }
    public DateTimeOffset CreatedAt { get; init; }
    public DateTimeOffset UpdatedAt { get; init; }
    public DateTimeOffset? DeletedAt { get; init; }
}

public sealed class CustomerContext
{
    public string Code { get; init; }
    public string Name { get; init; }
    public CustomerType Type { get; init; }
    public CustomerStatus Status { get; init; }
    public string Email { get; init; }
    public string? Phone { get; init; }
    public string? TaxNumber { get; init; }
    public decimal CreditLimit { get; init; }
    public decimal AvailableCredit { get; init; }
    public TaxType TaxType { get; init; }
    public decimal TaxRate { get; init; }
    public int PaymentTermsDays { get; init; }
    public string Currency { get; init; }
    public Guid? AssignedTo { get; init; }
    public Guid? PriceList { get; init; }
    public DateTimeOffset CreatedAt { get; init; }
    public DateTimeOffset UpdatedAt { get; init; }
    public DateTimeOffset? DeletedAt { get; init; }

    public UserContext? AssignedTo { get; init; }
    public PriceListContext? PriceList { get; init; }
}

public sealed class AddressContext
{
    public Guid Customer { get; init; }
    public AddressType Type { get; init; }
    public string Line1 { get; init; }
    public string? Line2 { get; init; }
    public string City { get; init; }
    public string? Region { get; init; }
    public string PostalCode { get; init; }
    public string Country { get; init; }
    public bool IsDefault { get; init; }
    public DateTimeOffset CreatedAt { get; init; }
    public DateTimeOffset UpdatedAt { get; init; }

    public CustomerContext? Customer { get; init; }
}

public sealed class ContactContext
{
    public Guid Customer { get; init; }
    public string FirstName { get; init; }
    public string LastName { get; init; }
    public string Email { get; init; }
    public string? Phone { get; init; }
    public bool IsPrimary { get; init; }
    public bool ReceivesInvoices { get; init; }
    public DateTimeOffset CreatedAt { get; init; }
    public DateTimeOffset UpdatedAt { get; init; }
    public DateTimeOffset? DeletedAt { get; init; }

    public CustomerContext? Customer { get; init; }
}

public sealed class ProductContext
{
    public string Sku { get; init; }
    public string Name { get; init; }
    public string? Description { get; init; }
    public ProductType Type { get; init; }
    public ProductStatus Status { get; init; }
    public decimal BasePrice { get; init; }
    public decimal? CostPrice { get; init; }
    public decimal? Margin { get; init; }
    public TaxType TaxType { get; init; }
    public int StockCount { get; init; }
    public int ReorderLevel { get; init; }
    public decimal? Weight { get; init; }
    public string? ImageUrl { get; init; }
    public DateTimeOffset CreatedAt { get; init; }
    public DateTimeOffset UpdatedAt { get; init; }
    public DateTimeOffset? DeletedAt { get; init; }
}

public sealed class PriceListContext
{
    public string Code { get; init; }
    public string Name { get; init; }
    public string Currency { get; init; }
    public DateOnly ValidFrom { get; init; }
    public DateOnly? ValidTo { get; init; }
    public bool Active { get; init; }
    public DateTimeOffset CreatedAt { get; init; }
    public DateTimeOffset UpdatedAt { get; init; }
    public DateTimeOffset? DeletedAt { get; init; }
}

public sealed class PriceListEntryContext
{
    public Guid PriceList { get; init; }
    public Guid Product { get; init; }
    public decimal Price { get; init; }
    public int MinQuantity { get; init; }
    public DateTimeOffset CreatedAt { get; init; }
    public DateTimeOffset UpdatedAt { get; init; }

    public PriceListContext? PriceList { get; init; }
    public ProductContext? Product { get; init; }
}

public sealed class OrderContext
{
    public string Reference { get; init; }
    public Guid Customer { get; init; }
    public OrderStatus Status { get; init; }
    public OrderPriority Priority { get; init; }
    public Guid? AssignedTo { get; init; }
    public Guid? ShippingAddress { get; init; }
    public Guid? BillingAddress { get; init; }
    public Guid? PriceList { get; init; }
    public string Currency { get; init; }
    public decimal Subtotal { get; init; }
    public decimal DiscountTotal { get; init; }
    public decimal TaxTotal { get; init; }
    public decimal ShippingCost { get; init; }
    public decimal Total { get; init; }
    public string? CustomerReference { get; init; }
    public string? Notes { get; init; }
    public DateOnly? RequestedDeliveryDate { get; init; }
    public DateTimeOffset? SubmittedAt { get; init; }
    public DateTimeOffset? ApprovedAt { get; init; }
    public Guid? ApprovedBy { get; init; }
    public DateTimeOffset CreatedAt { get; init; }
    public DateTimeOffset UpdatedAt { get; init; }

    public CustomerContext? Customer { get; init; }
    public UserContext? AssignedTo { get; init; }
    public AddressContext? ShippingAddress { get; init; }
    public AddressContext? BillingAddress { get; init; }
    public PriceListContext? PriceList { get; init; }
    public UserContext? ApprovedBy { get; init; }
}

public sealed class OrderLineContext
{
    public Guid Order { get; init; }
    public Guid Product { get; init; }
    public int Quantity { get; init; }
    public decimal UnitPrice { get; init; }
    public decimal DiscountPercent { get; init; }
    public decimal DiscountAmount { get; init; }
    public decimal TaxRate { get; init; }
    public decimal TaxAmount { get; init; }
    public decimal LineTotal { get; init; }
    public string? Notes { get; init; }

    public OrderContext? Order { get; init; }
    public ProductContext? Product { get; init; }
}

public sealed class DiscountContext
{
    public Guid Order { get; init; }
    public Guid? OrderLine { get; init; }
    public DiscountType Type { get; init; }
    public decimal Value { get; init; }
    public string Reason { get; init; }
    public Guid? ApprovedBy { get; init; }
    public DateTimeOffset CreatedAt { get; init; }
    public DateTimeOffset UpdatedAt { get; init; }

    public OrderContext? Order { get; init; }
    public OrderLineContext? OrderLine { get; init; }
    public UserContext? ApprovedBy { get; init; }
}

public sealed class ShipmentContext
{
    public Guid Order { get; init; }
    public ShipmentStatus Status { get; init; }
    public Guid ShippingAddress { get; init; }
    public string? TrackingNumber { get; init; }
    public string? Carrier { get; init; }
    public DateTimeOffset? DispatchedAt { get; init; }
    public DateOnly? EstimatedDelivery { get; init; }
    public DateTimeOffset? DeliveredAt { get; init; }
    public DateTimeOffset CreatedAt { get; init; }
    public DateTimeOffset UpdatedAt { get; init; }

    public OrderContext? Order { get; init; }
    public AddressContext? ShippingAddress { get; init; }
}

public sealed class ShipmentLineContext
{
    public Guid Shipment { get; init; }
    public Guid OrderLine { get; init; }
    public int QuantityShipped { get; init; }

    public ShipmentContext? Shipment { get; init; }
    public OrderLineContext? OrderLine { get; init; }
}

public sealed class InvoiceContext
{
    public string Reference { get; init; }
    public Guid Order { get; init; }
    public Guid Customer { get; init; }
    public InvoiceStatus Status { get; init; }
    public Guid BillingAddress { get; init; }
    public decimal Subtotal { get; init; }
    public decimal TaxTotal { get; init; }
    public decimal Total { get; init; }
    public decimal AmountPaid { get; init; }
    public decimal AmountOutstanding { get; init; }
    public DateOnly DueDate { get; init; }
    public DateTimeOffset? IssuedAt { get; init; }
    public DateTimeOffset? PaidAt { get; init; }
    public DateTimeOffset CreatedAt { get; init; }
    public DateTimeOffset UpdatedAt { get; init; }

    public OrderContext? Order { get; init; }
    public CustomerContext? Customer { get; init; }
    public AddressContext? BillingAddress { get; init; }
}

public sealed class PaymentContext
{
    public Guid Invoice { get; init; }
    public Guid Customer { get; init; }
    public PaymentStatus Status { get; init; }
    public PaymentMethod Method { get; init; }
    public decimal Amount { get; init; }
    public string Currency { get; init; }
    public string? Reference { get; init; }
    public DateTimeOffset? ProcessedAt { get; init; }
    public Guid? ProcessedBy { get; init; }
    public DateTimeOffset CreatedAt { get; init; }
    public DateTimeOffset UpdatedAt { get; init; }

    public InvoiceContext? Invoice { get; init; }
    public CustomerContext? Customer { get; init; }
    public UserContext? ProcessedBy { get; init; }
}

public sealed class CreditNoteContext
{
    public string Reference { get; init; }
    public Guid Customer { get; init; }
    public Guid? Invoice { get; init; }
    public decimal Amount { get; init; }
    public string Reason { get; init; }
    public Guid IssuedBy { get; init; }
    public Guid? AppliedToInvoice { get; init; }
    public DateTimeOffset CreatedAt { get; init; }
    public DateTimeOffset UpdatedAt { get; init; }

    public CustomerContext? Customer { get; init; }
    public InvoiceContext? Invoice { get; init; }
    public UserContext? IssuedBy { get; init; }
    public InvoiceContext? AppliedToInvoice { get; init; }
}
