# \IntermagnetLocationHandlerApi

All URIs are relative to *http://127.0.0.1:8070*

Method | HTTP request | Description
------------- | ------------- | -------------
[**get_intermagnet_location_handler**](IntermagnetLocationHandlerApi.md#get_intermagnet_location_handler) | **GET** /api/v1/intermagnet-locations/{id} | 
[**list_intermagnet_location_handler**](IntermagnetLocationHandlerApi.md#list_intermagnet_location_handler) | **GET** /api/v1/intermagnet-locations | 



## get_intermagnet_location_handler

> models::IntermagnetLocationExtendedModel get_intermagnet_location_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Intermagnet location id for fetching intermagnet location | [required] |

### Return type

[**models::IntermagnetLocationExtendedModel**](IntermagnetLocationExtendedModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## list_intermagnet_location_handler

> models::PagedResponseIntermagnetLocationExtendedModel list_intermagnet_location_handler(page, limit, order_by, desc, iaga, name, status, has_intermagnet)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**page** | Option<**i32**> | page number |  |
**limit** | Option<**i32**> | page size |  |
**order_by** | Option<**String**> | list order by |  |
**desc** | Option<**bool**> | should use descending instead |  |
**iaga** | Option<**String**> | iaga code |  |
**name** | Option<**String**> | intermagnet location name |  |
**status** | Option<[**IntermagnetLocationStatus**](.md)> | intermagnet location status |  |
**has_intermagnet** | Option<**bool**> | If true, only locations with at least one intermagnet are returned. If false, only locations without any intermagnets are returned. This filter is ignored if not provided. |  |

### Return type

[**models::PagedResponseIntermagnetLocationExtendedModel**](PagedResponse_IntermagnetLocationExtendedModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

